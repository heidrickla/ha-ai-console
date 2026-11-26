//! Request routing and the feed store. The store is the only writer of `/data/feeds`.

use std::collections::BTreeMap;
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use serde_json::{Value, json};

use crate::clock::{format_rfc3339, parse_rfc3339};
use crate::page::page;
use crate::validate::{Kind, validate, validate_fresh};

/// Wall-clock unix seconds, for the times in feeds.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;
/// Monotonic seconds, for scheduling: a step of the wall clock must not stall or speed up a poll.
pub type Mono = Arc<dyn Fn() -> u64 + Send + Sync>;

pub const MAX_BODY: usize = 4 * 1024 * 1024;
const KEEP_S: i64 = 24 * 3600;
const PAGE_ACTIVE_S: u64 = 120;
const SKEW_SHOWN_S: i64 = 60;
const FEEDS: [&str; 7] = [
    "quota", "meter", "routes", "router", "cloud", "health", "harness",
];

pub struct Config {
    pub data_dir: PathBuf,
    pub push_sources: Vec<IpAddr>,
    pub router_url: String,
    /// Most events kept in each of the routes and router stores.
    pub max_events: usize,
    /// Most events one `/api/feeds` reply carries per event feed, the newest.
    pub page_events: usize,
}

impl Config {
    pub fn new(data_dir: PathBuf, push_sources: Vec<IpAddr>, router_url: String) -> Config {
        Config {
            data_dir,
            push_sources,
            router_url,
            max_events: 20_000,
            page_events: 5_000,
        }
    }
}

pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Reply {
    fn json(status: u16, v: &Value) -> Reply {
        Reply {
            status,
            content_type: "application/json",
            body: v.to_string().into_bytes(),
        }
    }
    fn text(status: u16, s: &str) -> Reply {
        Reply {
            status,
            content_type: "text/plain; charset=utf-8",
            body: s.as_bytes().to_vec(),
        }
    }
    fn empty(status: u16) -> Reply {
        Reply {
            status,
            content_type: "text/plain",
            body: vec![],
        }
    }
}

fn kind_of(name: &str) -> Option<Kind> {
    Some(match name {
        "quota" => Kind::Quota,
        "meter" => Kind::Meter,
        "routes" => Kind::Routes,
        "router" => Kind::Router,
        "cloud" => Kind::Cloud,
        "health" => Kind::Health,
        _ => return None,
    })
}

fn is_event_feed(name: &str) -> bool {
    name == "routes" || name == "router"
}

fn event_time(e: &Value) -> i64 {
    e["t"].as_str().and_then(parse_rfc3339).unwrap_or(0)
}

#[derive(Default)]
struct Store {
    /// The latest document of each feed that is not an event feed.
    docs: BTreeMap<String, Arc<Value>>,
    /// Events of `routes` and `router` by id, the merge behind each feed.
    events: BTreeMap<String, BTreeMap<String, Arc<Value>>>,
    /// `at` and `source` of the last accepted event document.
    meta: BTreeMap<String, Value>,
    errors: BTreeMap<String, String>,
}

pub struct App {
    cfg: Config,
    clock: Clock,
    mono: Mono,
    store: Mutex<Store>,
    /// Serializes file writes, so the file always holds the newest snapshot.
    io: Mutex<()>,
    /// Monotonic second of the last `/api/feeds` read, plus one; 0 means never.
    last_poll: AtomicU64,
    discarded: Vec<String>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn check_loaded(name: &str, doc: &Value) -> Result<(), String> {
    match kind_of(name) {
        Some(kind) => validate(kind, doc),
        None if doc["agents"].is_array() => Ok(()),
        None => Err("no agents list".into()),
    }
}

impl App {
    pub fn new(cfg: Config, clock: Clock) -> App {
        let start = Instant::now();
        App::with_mono(cfg, clock, Arc::new(move || start.elapsed().as_secs()))
    }

    pub fn with_mono(cfg: Config, clock: Clock, mono: Mono) -> App {
        let mut store = Store::default();
        let mut discarded = Vec::new();
        let mut loaded_events: BTreeMap<String, BTreeMap<String, Arc<Value>>> = BTreeMap::new();
        for name in FEEDS {
            let path = cfg.data_dir.join("feeds").join(format!("{name}.json"));
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    discarded.push(format!("{name}.json: unreadable: {e}"));
                    continue;
                }
            };
            let loaded = serde_json::from_str::<Value>(&text)
                .map_err(|e| {
                    if text.is_empty() {
                        "empty file".to_string()
                    } else {
                        format!("not JSON: {e}")
                    }
                })
                .and_then(|doc| check_loaded(name, &doc).map(|_| doc));
            match loaded {
                Ok(doc) if is_event_feed(name) => {
                    for e in doc["events"].as_array().into_iter().flatten() {
                        loaded_events.entry(name.to_string()).or_default().insert(
                            e["id"].as_str().unwrap_or_default().to_string(),
                            Arc::new(e.clone()),
                        );
                    }
                    store.meta.insert(
                        name.to_string(),
                        json!({"at": doc["at"], "source": doc["source"]}),
                    );
                }
                Ok(doc) => {
                    store.docs.insert(name.to_string(), Arc::new(doc));
                }
                Err(why) => discarded.push(format!("{name}.json: {why}")),
            }
        }
        for note in &discarded {
            eprintln!("ai-console: discarded stored feed {note}");
        }
        let app = App {
            cfg,
            clock,
            mono,
            store: Mutex::new(Store::default()),
            io: Mutex::new(()),
            last_poll: AtomicU64::new(0),
            discarded,
        };
        // The same age and size limits as a push applies, so a stored file over the cap comes back capped.
        for (name, map) in loaded_events {
            store.events.insert(name, app.trimmed(map));
        }
        *lock(&app.store) = store;
        app
    }

    /// Stored feed files that could not be read back at start, each with its reason.
    pub fn discarded(&self) -> &[String] {
        &self.discarded
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    pub fn router_url(&self) -> &str {
        &self.cfg.router_url
    }

    /// The events kept: nothing older than 24 hours, and at most `max_events`, the newest.
    fn trimmed(&self, mut map: BTreeMap<String, Arc<Value>>) -> BTreeMap<String, Arc<Value>> {
        let cutoff = self.now() - KEEP_S;
        map.retain(|_, e| event_time(e) >= cutoff);
        if map.len() > self.cfg.max_events {
            let mut by_age: Vec<(i64, String)> = map
                .iter()
                .map(|(id, e)| (event_time(e), id.clone()))
                .collect();
            by_age.sort();
            for (_, id) in by_age.into_iter().take(map.len() - self.cfg.max_events) {
                map.remove(&id);
            }
        }
        map
    }

    fn event_bytes(
        map: &BTreeMap<String, Arc<Value>>,
        meta: &Value,
    ) -> serde_json::Result<Vec<u8>> {
        let mut evs: Vec<&Arc<Value>> = map.values().collect();
        evs.sort_by_key(|e| {
            (
                event_time(e),
                e["id"].as_str().unwrap_or_default().to_string(),
            )
        });
        let refs: Vec<&Value> = evs.iter().map(|e| &***e).collect();
        let mut doc = json!({"v": 1, "at": meta["at"], "events": refs});
        if !meta["source"].is_null() {
            doc["source"] = meta["source"].clone();
        }
        serde_json::to_vec(&doc)
    }

    /// Take a validated document. The new state is built aside and written first; it replaces the
    /// stored one only when the write worked or `durable` is false (a pulled feed is shown even when
    /// it cannot be saved, with the failure on the card). Err names why the snapshot was not saved.
    fn accept(&self, name: &str, doc: Value, durable: bool) -> Result<(), String> {
        let _io = lock(&self.io);
        enum Next {
            Events(BTreeMap<String, Arc<Value>>, Value),
            Doc(Arc<Value>),
        }
        let (next, bytes) = if is_event_feed(name) {
            let mut map = lock(&self.store)
                .events
                .get(name)
                .cloned()
                .unwrap_or_default();
            for e in doc["events"].as_array().into_iter().flatten() {
                if let Some(id) = e["id"].as_str() {
                    map.insert(id.to_string(), Arc::new(e.clone()));
                }
            }
            let map = self.trimmed(map);
            let meta = json!({"at": doc["at"], "source": doc.get("source")});
            let bytes = Self::event_bytes(&map, &meta);
            (Next::Events(map, meta), bytes)
        } else {
            let bytes = serde_json::to_vec(&doc);
            (Next::Doc(Arc::new(doc)), bytes)
        };
        let written = bytes
            .map_err(std::io::Error::other)
            .and_then(|b| write_durably(&self.cfg.data_dir.join("feeds"), name, &b));
        let failure = written.err().map(|e| {
            eprintln!("ai-console: cannot write feed {name}: {e}");
            format!("cannot save {name}: {e}")
        });
        if let (Some(f), true) = (&failure, durable) {
            lock(&self.store).errors.insert(name.to_string(), f.clone());
            return Err(f.clone());
        }
        let mut st = lock(&self.store);
        match next {
            Next::Events(map, meta) => {
                st.events.insert(name.to_string(), map);
                st.meta.insert(name.to_string(), meta);
            }
            Next::Doc(d) => {
                st.docs.insert(name.to_string(), d);
            }
        }
        match &failure {
            Some(f) => st.errors.insert(name.to_string(), f.clone()),
            None => st.errors.remove(name),
        };
        Ok(())
    }

    pub fn ingest_pulled(&self, name: &str, body: &str) -> Result<(), String> {
        let result = (|| {
            let kind = match kind_of(name) {
                Some(k @ (Kind::Router | Kind::Cloud | Kind::Health)) => k,
                _ => return Err(format!("{name} is not a pulled feed")),
            };
            let mut doc: Value =
                serde_json::from_str(body).map_err(|e| format!("not JSON: {e}"))?;
            validate(kind, &doc)?;
            validate_fresh(kind, &doc, self.now())?;
            if kind != Kind::Router {
                doc["at"] = json!(format_rfc3339(self.now()));
            }
            Ok(doc)
        })();
        match result {
            Ok(doc) => self.accept(name, doc, false),
            Err(e) => {
                self.pull_failed(name, &e);
                Err(e)
            }
        }
    }

    pub fn ingest_harness(&self, mut doc: Value) {
        doc["at"] = json!(format_rfc3339(self.now()));
        let _ = self.accept("harness", doc, false);
    }

    pub fn pull_failed(&self, name: &str, err: &str) {
        lock(&self.store)
            .errors
            .insert(name.to_string(), err.to_string());
    }

    /// Where the next router pull starts: a minute before the router's own last `at`, within 24 hours.
    /// Event times never enter it, so a wrong event cannot move the pull.
    pub fn router_since(&self) -> i64 {
        let now = self.now();
        let at = lock(&self.store)
            .meta
            .get("router")
            .and_then(|m| m["at"].as_str().and_then(parse_rfc3339));
        at.map_or(now - KEEP_S, |t| (t.min(now) - 60).max(now - KEEP_S))
    }

    pub fn poll_interval_s(&self) -> u64 {
        let last = self.last_poll.load(Ordering::SeqCst);
        if last != 0 && (self.mono)().saturating_sub(last - 1) <= PAGE_ACTIVE_S {
            15
        } else {
            60
        }
    }

    fn feeds_reply(&self) -> Reply {
        let (now, cutoff) = (self.now(), self.now() - KEEP_S);
        self.last_poll.store((self.mono)() + 1, Ordering::SeqCst);
        struct Snap {
            doc: Option<Arc<Value>>,
            events: Option<(Vec<Arc<Value>>, usize)>,
            meta: Option<Value>,
            error: Option<String>,
        }
        let snaps: Vec<(&str, Snap)> = {
            let st = lock(&self.store);
            FEEDS
                .iter()
                .map(|&name| {
                    let events = st.events.get(name).map(|m| {
                        let mut live: Vec<Arc<Value>> = m
                            .values()
                            .filter(|e| event_time(e) >= cutoff)
                            .cloned()
                            .collect();
                        let total = live.len();
                        live.sort_by_key(|e| {
                            (
                                event_time(e),
                                e["id"].as_str().unwrap_or_default().to_string(),
                            )
                        });
                        let skip = total.saturating_sub(self.cfg.page_events);
                        (live.split_off(skip), total)
                    });
                    (
                        name,
                        Snap {
                            doc: st.docs.get(name).cloned(),
                            events,
                            meta: st.meta.get(name).cloned(),
                            error: st.errors.get(name).cloned(),
                        },
                    )
                })
                .collect()
        };
        let mut feeds = serde_json::Map::new();
        for (name, s) in snaps {
            let (data, at, total) = match (&s.doc, &s.events) {
                (Some(d), _) => (
                    Some((**d).clone()),
                    d["at"].as_str().and_then(parse_rfc3339),
                    None,
                ),
                (None, Some((evs, total))) => {
                    let m = s.meta.clone().unwrap_or_default();
                    let mut d = json!({"v": 1, "at": m["at"], "events": evs.iter().map(|e| &**e).collect::<Vec<&Value>>()});
                    if !m["source"].is_null() {
                        d["source"] = m["source"].clone();
                    }
                    (
                        Some(d),
                        m["at"].as_str().and_then(parse_rfc3339),
                        Some(*total),
                    )
                }
                (None, None) => (None, None, None),
            };
            let ahead = at.map(|t| t - now).filter(|d| *d > SKEW_SHOWN_S);
            feeds.insert(
                name.to_string(),
                json!({
                    "present": data.is_some(),
                    "age_s": at.map(|t| (now - t).max(0)),
                    "skew_s": ahead,
                    "error": s.error,
                    "events_total": total,
                    "data": data,
                }),
            );
        }
        Reply::json(200, &json!({"now": format_rfc3339(now), "feeds": feeds}))
    }

    /// The answer that needs no body, given the request line, the sender and the declared length:
    /// refusals come before a byte of the body is read. None means the request may go on.
    pub fn precheck(
        &self,
        method: &str,
        url: &str,
        remote: Option<IpAddr>,
        content_length: Option<usize>,
    ) -> Option<Reply> {
        let path = url.split('?').next().unwrap_or("");
        if let Some(name) = path.strip_prefix("/api/feed/") {
            if method != "POST" {
                return Some(Reply::json(405, &json!({"error": "POST only"})));
            }
            if !matches!(name, "quota" | "meter" | "routes") {
                return Some(Reply::json(404, &json!({"error": "unknown feed"})));
            }
            let seen = remote.map_or("unknown".to_string(), |a| a.to_string());
            if !remote.is_some_and(|a| self.cfg.push_sources.contains(&a)) {
                eprintln!("ai-console: push {name} refused from {seen}");
                return Some(Reply::json(
                    403,
                    &json!({"error": format!("{seen} is not in push_sources")}),
                ));
            }
            return match content_length {
                None => Some(Reply::json(
                    411,
                    &json!({"error": "Content-Length is required"}),
                )),
                Some(n) if n > MAX_BODY => {
                    Some(Reply::json(413, &json!({"error": "body is over 4 MiB"})))
                }
                Some(_) => None,
            };
        }
        match (method, path) {
            ("GET" | "HEAD", "/" | "/healthz" | "/api/feeds") => None,
            ("GET" | "HEAD", _) => Some(Reply::text(404, "not found")),
            _ => Some(Reply::json(405, &json!({"error": "method not allowed"}))),
        }
    }

    fn push(&self, name: &str, remote: Option<IpAddr>, body: &[u8]) -> Reply {
        let kind = kind_of(name).unwrap_or(Kind::Meter);
        let outcome = serde_json::from_slice::<Value>(body)
            .map_err(|e| format!("body is not JSON: {e}"))
            .and_then(|doc| validate(kind, &doc).map(|_| doc))
            .and_then(|doc| validate_fresh(kind, &doc, self.now()).map(|_| doc));
        match outcome {
            Ok(doc) => match self.accept(name, doc, true) {
                Ok(()) => {
                    eprintln!(
                        "ai-console: push {name} accepted from {}",
                        remote.map_or("unknown".into(), |a| a.to_string())
                    );
                    Reply::empty(204)
                }
                Err(e) => Reply::json(500, &json!({"error": e})),
            },
            Err(e) => {
                self.pull_failed(name, &e);
                Reply::json(400, &json!({"error": e}))
            }
        }
    }

    pub fn handle(&self, method: &str, url: &str, remote: Option<IpAddr>, body: &[u8]) -> Reply {
        if let Some(refusal) = self.precheck(method, url, remote, Some(body.len())) {
            return refusal;
        }
        let path = url.split('?').next().unwrap_or("");
        match path {
            "/" => Reply {
                status: 200,
                content_type: "text/html; charset=utf-8",
                body: page().into_bytes(),
            },
            "/healthz" => Reply::text(200, "ok"),
            "/api/feeds" => self.feeds_reply(),
            p => self.push(&p["/api/feed/".len()..], remote, body),
        }
    }
}

fn write_durably(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{name}.json.tmp"));
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, dir.join(format!("{name}.json")))?;
    // A directory cannot be opened for sync on Windows; the add-on runs on Linux.
    #[cfg(unix)]
    std::fs::File::open(dir)?.sync_all()?;
    Ok(())
}
