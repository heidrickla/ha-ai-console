//! The HTTP side on hyper 1.x (HTTP/1 only) over tokio: one request per connection, every phase under
//! a deadline, and the source and size refusals answered from the parsed head before the body is polled.

use std::io;
use std::net::{IpAddr, TcpListener};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::thread::JoinHandle;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::header::{CONTENT_LENGTH, HeaderValue, TRANSFER_ENCODING};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;

use crate::app::{App, MAX_BODY, Reply};

const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; img-src data:; base-uri 'none'; form-action 'none'";
/// The request line and all headers together; hyper answers 431 beyond it.
const MAX_HEAD: usize = 16 * 1024;
const MAX_HEADERS: usize = 64;
/// Connections held at once; one over the cap is closed unanswered.
const MAX_CONNECTIONS: usize = 4096;

#[derive(Clone)]
pub struct ServerConfig {
    /// The whole request line and headers must arrive within this.
    pub head_timeout: Duration,
    /// The whole body must arrive within this.
    pub body_timeout: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            head_timeout: Duration::from_secs(5),
            body_timeout: Duration::from_secs(30),
        }
    }
}

/// Reads the connection, and turns a line feed that no carriage return precedes into a control byte
/// until the end of the head, so hyper refuses the request instead of reading it differently from a
/// proxy in front. The body is passed through untouched.
struct HeadGuard {
    inner: TcpStream,
    in_head: bool,
    prev: u8,
    tail: [u8; 4],
}

impl AsyncRead for HeadGuard {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let start = buf.filled().len();
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
        if !matches!(polled, Poll::Ready(Ok(()))) || !self.in_head {
            return polled;
        }
        let this = &mut *self;
        for b in &mut buf.filled_mut()[start..] {
            if !this.in_head {
                break;
            }
            if *b == b'\n' && this.prev != b'\r' {
                *b = 1;
            }
            this.prev = *b;
            this.tail = [this.tail[1], this.tail[2], this.tail[3], *b];
            if &this.tail == b"\r\n\r\n" {
                this.in_head = false;
            }
        }
        polled
    }
}

impl AsyncWrite for HeadGuard {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        b: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, b)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// Run the server on its own thread; the handle ends only if the runtime or the listener fails.
pub fn spawn(app: Arc<App>, listener: TcpListener, cfg: ServerConfig) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(16)
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => return eprintln!("ai-console: cannot start the runtime: {e}"),
        };
        rt.block_on(accept_loop(app, listener, cfg));
    })
}

async fn accept_loop(app: Arc<App>, listener: TcpListener, cfg: ServerConfig) {
    let listener = match listener
        .set_nonblocking(true)
        .and_then(|()| tokio::net::TcpListener::from_std(listener))
    {
        Ok(l) => l,
        Err(e) => return eprintln!("ai-console: cannot use the listener: {e}"),
    };
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(a) => a,
            Err(e) => {
                eprintln!("ai-console: accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            continue;
        };
        let (app, cfg) = (app.clone(), cfg.clone());
        tokio::spawn(async move {
            // Head, body and the answer each have a deadline of their own; this one ends what is left.
            let whole = cfg.head_timeout + cfg.body_timeout + Duration::from_secs(15);
            let _ = tokio::time::timeout(whole, serve(app, stream, peer.ip(), cfg)).await;
            drop(slot);
        });
    }
}

async fn serve(app: Arc<App>, stream: TcpStream, remote: IpAddr, cfg: ServerConfig) {
    let io = TokioIo::new(HeadGuard {
        inner: stream,
        in_head: true,
        prev: 0,
        tail: [0; 4],
    });
    let service = service_fn(move |req| {
        let (app, body_timeout) = (app.clone(), cfg.body_timeout);
        async move { Ok::<_, std::convert::Infallible>(answer(app, req, remote, body_timeout).await) }
    });
    let mut conn = http1::Builder::new();
    conn.timer(TokioTimer::new())
        .header_read_timeout(cfg.head_timeout)
        .keep_alive(false)
        .max_buf_size(MAX_HEAD)
        .max_headers(MAX_HEADERS);
    let _ = conn.serve_connection(io, service).await;
}

fn reason(status: u16) -> &'static str {
    match status {
        400 => "Bad Request",
        408 => "Request Timeout",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        _ => "Error",
    }
}

fn response(reply: Reply) -> Response<Full<Bytes>> {
    let mut r = Response::new(Full::new(Bytes::from(reply.body)));
    *r.status_mut() = hyper::StatusCode::from_u16(reply.status)
        .unwrap_or(hyper::StatusCode::INTERNAL_SERVER_ERROR);
    let h = r.headers_mut();
    h.insert("content-type", HeaderValue::from_static(reply.content_type));
    h.insert("cache-control", HeaderValue::from_static("no-store"));
    h.insert("content-security-policy", HeaderValue::from_static(CSP));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    r
}

fn plain(status: u16) -> Response<Full<Bytes>> {
    response(Reply {
        status,
        content_type: "text/plain",
        body: format!("{status} {}\n", reason(status)).into_bytes(),
    })
}

/// The declared body length, None when absent, unparsable or sent chunked.
fn declared_length(req: &Request<Incoming>) -> Option<usize> {
    if req.headers().contains_key(TRANSFER_ENCODING) {
        return None;
    }
    req.headers()
        .get(CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .parse()
        .ok()
}

async fn answer(
    app: Arc<App>,
    req: Request<Incoming>,
    remote: IpAddr,
    body_timeout: Duration,
) -> Response<Full<Bytes>> {
    let method = req.method().as_str().to_string();
    let target = req
        .uri()
        .path_and_query()
        .map_or("/", |p| p.as_str())
        .to_string();
    let is_post = method == "POST";
    // Decided from the parsed head alone: the body is not polled for a request that is refused.
    let length = if is_post {
        declared_length(&req)
    } else {
        Some(0)
    };
    let checked = {
        let (app, method, target) = (app.clone(), method.clone(), target.clone());
        tokio::task::spawn_blocking(move || app.precheck(&method, &target, Some(remote), length))
            .await
            .ok()
            .flatten()
    };
    if let Some(refusal) = checked {
        return response(refusal);
    }
    let mut body = Bytes::new();
    if is_post {
        let limited = Limited::new(req.into_body(), MAX_BODY);
        match tokio::time::timeout(body_timeout, limited.collect()).await {
            Ok(Ok(all)) => body = all.to_bytes(),
            Ok(Err(e))
                if e.downcast_ref::<http_body_util::LengthLimitError>()
                    .is_some() =>
            {
                return plain(413);
            }
            Ok(Err(_)) => return plain(400),
            Err(_) => return plain(408),
        }
    }
    let handled =
        tokio::task::spawn_blocking(move || app.handle(&method, &target, Some(remote), &body))
            .await;
    match handled {
        Ok(reply) => response(reply),
        Err(_) => plain(500),
    }
}
