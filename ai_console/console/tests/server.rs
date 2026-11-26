//! The real listener on 127.0.0.1: what the page's host and a pushing client see on the wire.
mod common;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use ai_console::server::{ServerConfig, spawn};
use common::*;

struct Wire {
    addr: SocketAddr,
    rig: Rig,
}

fn start(tag: &str, sources: &[&str]) -> Wire {
    start_with(
        tag,
        sources,
        Duration::from_millis(600),
        Duration::from_millis(600),
    )
}

fn start_with(tag: &str, sources: &[&str], head: Duration, body: Duration) -> Wire {
    let sources: Vec<std::net::IpAddr> = sources.iter().map(|s| s.parse().unwrap()).collect();
    let rig = rig_with(tag, |c| c.push_sources = sources);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    spawn(
        rig.app.clone(),
        listener,
        ServerConfig {
            head_timeout: head,
            body_timeout: body,
        },
    );
    Wire { addr, rig }
}

struct Answer {
    status: u16,
    head: String,
    body: Vec<u8>,
}

fn read_answer(mut s: TcpStream) -> Option<Answer> {
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut all = Vec::new();
    let _ = s.read_to_end(&mut all);
    let split = all.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&all[..split]).into_owned();
    let status = head.split(' ').nth(1)?.parse().ok()?;
    Some(Answer {
        status,
        head,
        body: all[split + 4..].to_vec(),
    })
}

fn connect(w: &Wire) -> TcpStream {
    TcpStream::connect(w.addr).unwrap()
}

fn one_shot(w: &Wire, raw: &[u8]) -> Answer {
    let mut s = connect(w);
    s.write_all(raw).unwrap();
    read_answer(s).expect("an answer")
}

fn header<'a>(a: &'a Answer, name: &str) -> Option<&'a str> {
    a.head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.eq_ignore_ascii_case(name).then_some(v.trim())
    })
}

#[test]
fn the_page_carries_its_security_headers() {
    let w = start("wire-get", &["127.0.0.1"]);
    let a = one_shot(&w, b"GET /?ingress=1 HTTP/1.1\r\nHost: x\r\n\r\n");
    assert_eq!(a.status, 200);
    assert!(header(&a, "content-type").unwrap().starts_with("text/html"));
    assert!(
        header(&a, "content-security-policy")
            .unwrap()
            .contains("default-src 'none'")
    );
    assert_eq!(header(&a, "x-content-type-options"), Some("nosniff"));
    assert_eq!(header(&a, "cache-control"), Some("no-store"));
    assert_eq!(
        header(&a, "content-length")
            .unwrap()
            .parse::<usize>()
            .unwrap(),
        a.body.len()
    );
    assert_eq!(a.body, ai_console::page::page().into_bytes());
}

#[test]
fn a_listed_source_pushes_a_feed_over_the_wire() {
    let w = start("wire-push", &["127.0.0.1"]);
    let body = fixture("meter.json");
    let raw = format!(
        "POST /api/feed/meter HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    assert_eq!(one_shot(&w, raw.as_bytes()).status, 204);
    assert!(w.rig.dir.join("feeds/meter.json").is_file());
    let bad = r#"{"v":1}"#;
    let raw = format!(
        "POST /api/feed/meter HTTP/1.1\r\nContent-Length: {}\r\n\r\n{bad}",
        bad.len()
    );
    let a = one_shot(&w, raw.as_bytes());
    assert_eq!(a.status, 400);
    assert!(String::from_utf8_lossy(&a.body).contains("`at`"));
}

#[test]
fn an_address_outside_push_sources_is_refused_before_any_body_is_read() {
    let w = start("wire-403", &["192.0.2.10"]);
    let mut s = connect(&w);
    s.write_all(b"POST /api/feed/meter HTTP/1.1\r\nContent-Length: 4194304\r\n\r\n")
        .unwrap();
    let started = Instant::now();
    let a = read_answer(s).expect("an answer without a body being sent");
    assert_eq!(a.status, 403);
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "refused only after {:?}",
        started.elapsed()
    );
    assert!(!w.rig.dir.join("feeds/meter.json").exists());
    assert_eq!(
        one_shot(&w, b"GET /healthz HTTP/1.1\r\n\r\n").status,
        200,
        "the only worker is free"
    );
}

#[test]
fn an_oversized_content_length_is_413_without_reading_it() {
    let w = start("wire-413", &["127.0.0.1"]);
    let mut s = connect(&w);
    s.write_all(b"POST /api/feed/meter HTTP/1.1\r\nContent-Length: 5242880\r\n\r\n")
        .unwrap();
    let started = Instant::now();
    let a = read_answer(s).expect("an answer");
    assert_eq!(a.status, 413);
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(!w.rig.dir.join("feeds/meter.json").exists());
}

#[test]
fn a_body_that_stalls_gets_408_and_frees_the_worker() {
    let w = start("wire-slow", &["127.0.0.1"]);
    let mut slow = connect(&w);
    slow.write_all(b"POST /api/feed/meter HTTP/1.1\r\nContent-Length: 1000\r\n\r\n0123456789")
        .unwrap();
    let started = Instant::now();
    let next = std::thread::spawn({
        let addr = w.addr;
        move || {
            std::thread::sleep(Duration::from_millis(100));
            let mut s = TcpStream::connect(addr).unwrap();
            s.write_all(b"GET /healthz HTTP/1.1\r\n\r\n").unwrap();
            let a = read_answer(s).expect("served after the stalled body timed out");
            (a.status, Instant::now())
        }
    });
    let a = read_answer(slow).expect("408 for the stalled body");
    assert_eq!(a.status, 408);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    let (status, _) = next.join().unwrap();
    assert_eq!(status, 200);
    assert!(!w.rig.dir.join("feeds/meter.json").exists());
}

#[test]
fn a_trickled_head_is_cut_off_at_the_deadline_not_per_read() {
    let w = start("wire-trickle", &["127.0.0.1"]);
    let mut s = connect(&w);
    let started = Instant::now();
    for b in b"GET /healthz HTTP/1.1\r\nX: " {
        if s.write_all(&[*b]).is_err() {
            break;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    let a = read_answer(s);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert!(a.is_none_or(|a| a.status == 408));
}

#[test]
fn unusual_requests_get_plain_refusals() {
    let w = start("wire-odd", &["127.0.0.1"]);
    assert_eq!(
        one_shot(&w, b"POST /api/feed/meter HTTP/1.1\r\n\r\n").status,
        411
    );
    assert_eq!(
        one_shot(
            &w,
            b"POST /api/feed/meter HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n"
        )
        .status,
        411
    );
    assert_eq!(one_shot(&w, b"DELETE / HTTP/1.1\r\n\r\n").status, 405);
    assert_eq!(one_shot(&w, b"GET /nothing HTTP/1.1\r\n\r\n").status, 404);
    assert_eq!(one_shot(&w, b"garbage\r\n\r\n").status, 400);
    let big = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(20000));
    assert_eq!(one_shot(&w, big.as_bytes()).status, 431);
    let head = one_shot(&w, b"HEAD / HTTP/1.1\r\n\r\n");
    assert_eq!((head.status, head.body.len()), (200, 0));
}

#[test]
fn expect_continue_is_answered_once_the_push_is_acceptable() {
    let w = start("wire-100", &["127.0.0.1"]);
    let mut s = connect(&w);
    let body = fixture("meter.json");
    s.write_all(
        format!(
            "POST /api/feed/meter HTTP/1.1\r\nExpect: 100-continue\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .as_bytes(),
    )
    .unwrap();
    let mut buf = [0u8; 64];
    s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let n = s.read(&mut buf).unwrap();
    assert!(String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 100"));
    s.write_all(body.as_bytes()).unwrap();
    assert_eq!(read_answer(s).unwrap().status, 204);
}

fn push_raw(w: &Wire, head_and_body: &[u8]) -> Option<u16> {
    let mut s = connect(w);
    s.write_all(head_and_body).unwrap();
    read_answer(s).map(|a| a.status)
}

#[test]
fn conflicting_content_lengths_are_refused_and_store_nothing() {
    let w = start("wire-dup-cl", &["127.0.0.1"]);
    let body = fixture("meter.json");
    // the first length is the whole body, the second is shorter: a last-wins reader would cut the body short
    let raw = format!(
        "POST /api/feed/meter HTTP/1.1\r\nContent-Length: {}\r\nContent-Length: 5\r\n\r\n{body}",
        body.len()
    );
    assert_eq!(push_raw(&w, raw.as_bytes()), Some(400));
    assert!(!w.rig.dir.join("feeds/meter.json").exists());
    // two equal lengths are one length
    let raw = format!(
        "POST /api/feed/meter HTTP/1.1\r\nContent-Length: {0}\r\nContent-Length: {0}\r\n\r\n{body}",
        body.len()
    );
    assert_eq!(push_raw(&w, raw.as_bytes()), Some(204));
}

#[test]
fn a_bare_lf_in_the_head_is_refused_so_no_proxy_can_disagree_about_where_it_ends() {
    let w = start("wire-bare-lf", &["127.0.0.1"]);
    let body = fixture("meter.json");
    let raw = format!(
        "POST /api/feed/meter HTTP/1.1\nContent-Length: {}\n\n{body}",
        body.len()
    );
    let status = push_raw(&w, raw.as_bytes());
    assert!(status.is_none_or(|s| s == 400), "{status:?}");
    assert!(!w.rig.dir.join("feeds/meter.json").exists());
    // a line feed inside the body is not part of the head
    let json = "{\"v\":1,\n\"at\":1}";
    let raw = format!(
        "POST /api/feed/meter HTTP/1.1\r\nContent-Length: {}\r\n\r\n{json}",
        json.len()
    );
    assert_eq!(
        push_raw(&w, raw.as_bytes()),
        Some(400),
        "reaches validation, not the framing guard"
    );
    let got = String::from_utf8_lossy(&one_shot(&w, raw.as_bytes()).body).into_owned();
    assert!(got.contains("`at`"), "{got}");
}

#[test]
fn an_unlisted_senders_body_is_not_read_before_the_refusal() {
    let w = start("wire-403-body", &["192.0.2.10"]);
    let mut s = connect(&w);
    let started = Instant::now();
    s.write_all(b"POST /api/feed/meter HTTP/1.1\r\nContent-Length: 4194304\r\n\r\n")
        .unwrap();
    // the body would be 4 MiB; send only a little and expect the answer without the rest
    let _ = s.write_all(&vec![b'x'; 8192]);
    let a = read_answer(s).expect("refusal");
    assert_eq!(a.status, 403);
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[test]
fn two_hundred_stalled_connections_do_not_keep_the_page_from_answering() {
    let w = start_with(
        "wire-flood",
        &["192.0.2.10"],
        Duration::from_secs(5),
        Duration::from_secs(5),
    );
    let mut held = Vec::new();
    for i in 0..200 {
        let mut s = connect(&w);
        let part: &[u8] = match i % 3 {
            0 => b"GET / HT",
            1 => b"POST /api/feed/meter HTTP/1.1\r\nContent-Length: 4194304\r\n",
            _ => b"POST /api/feed/meter HTTP/1.1\r\nContent-Length: 4194304\r\n\r\n",
        };
        s.write_all(part).unwrap();
        held.push(s);
    }
    let started = Instant::now();
    let a = one_shot(&w, b"GET /healthz HTTP/1.1\r\n\r\n");
    assert_eq!(a.status, 200);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    drop(held);
}
