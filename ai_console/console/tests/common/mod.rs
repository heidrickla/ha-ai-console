#![allow(dead_code)]
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use ai_console::app::{App, Config, Reply};
use serde_json::Value;

pub const NOW: i64 = 1_794_902_430; // 2026-11-17T08:00:30Z

pub fn fixture(name: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

pub fn fixture_json(name: &str) -> Value {
    serde_json::from_str(&fixture(name)).unwrap()
}

pub struct Rig {
    pub app: Arc<App>,
    pub dir: PathBuf,
    pub clock: Arc<AtomicI64>,
    pub mono: Arc<AtomicU64>,
}

/// A rig's directory goes with it, so a run leaves the temp directory as it found it.
impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub fn dir_for(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aiconsole-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn rig(tag: &str) -> Rig {
    rig_with(tag, |_| {})
}

pub fn rig_with(tag: &str, tweak: impl FnOnce(&mut Config)) -> Rig {
    let dir = dir_for(tag);
    build(dir, tweak)
}

pub fn build(dir: PathBuf, tweak: impl FnOnce(&mut Config)) -> Rig {
    let clock = Arc::new(AtomicI64::new(NOW));
    let mono = Arc::new(AtomicU64::new(1000));
    let (c, m) = (clock.clone(), mono.clone());
    let mut cfg = Config::new(
        dir.clone(),
        vec!["192.0.2.10".parse().unwrap()],
        "http://router.invalid:8789".into(),
    );
    tweak(&mut cfg);
    let app = Arc::new(App::with_mono(
        cfg,
        Arc::new(move || c.load(Ordering::SeqCst)),
        Arc::new(move || m.load(Ordering::SeqCst)),
    ));
    Rig {
        app,
        dir,
        clock,
        mono,
    }
}

pub fn workstation() -> Option<IpAddr> {
    Some("192.0.2.10".parse().unwrap())
}

pub fn post(r: &Rig, name: &str, body: &str, from: Option<IpAddr>) -> Reply {
    r.app
        .handle("POST", &format!("/api/feed/{name}"), from, body.as_bytes())
}

pub fn get(r: &Rig, url: &str) -> Reply {
    r.app
        .handle("GET", url, Some("127.0.0.1".parse().unwrap()), b"")
}

pub fn text(r: &Reply) -> String {
    String::from_utf8_lossy(&r.body).into_owned()
}

pub fn json(r: &Reply) -> Value {
    serde_json::from_slice(&r.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", text(r)))
}
