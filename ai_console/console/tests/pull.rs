//! The pullers against a stand-in upstream on 127.0.0.1.
mod common;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ai_console::pull::{agent, pull_harness, pull_router_side};
use common::*;
use serde_json::json;

type Routes = HashMap<String, (u16, String)>;

struct Upstream {
    base: String,
    seen: Arc<Mutex<Vec<String>>>,
}

fn upstream(routes: Routes) -> Upstream {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                match s.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
            let head = String::from_utf8_lossy(&buf).into_owned();
            // take a declared body too, so the client is not reset while still sending it
            let wanted = head
                .to_ascii_lowercase()
                .lines()
                .find_map(|l| {
                    l.strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            let have = buf.len()
                - buf
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map_or(0, |p| p + 4);
            let mut rest = wanted.saturating_sub(have);
            while rest > 0 {
                match s.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => rest = rest.saturating_sub(n),
                }
            }
            let line = head.lines().next().unwrap_or("").to_string();
            let target = line.split(' ').nth(1).unwrap_or("").to_string();
            let authed = head
                .to_ascii_lowercase()
                .contains("authorization: bearer tok");
            log.lock().unwrap().push(format!("{line} auth={authed}"));
            let path = target.split('?').next().unwrap_or("");
            let (status, body) = match routes.get(path) {
                Some((401, body)) if authed => (200, body.clone()),
                Some((401, _)) => (401, "{}".to_string()),
                Some((code, body)) => (*code, body.clone()),
                None => (404, "{}".to_string()),
            };
            let _ = write!(
                s,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    Upstream { base, seen }
}

fn router_routes() -> Routes {
    [
        ("/routes", "router.json"),
        ("/cloud", "cloud.json"),
        ("/health", "health.json"),
    ]
    .into_iter()
    .map(|(p, f)| (p.to_string(), (200, fixture(f))))
    .collect()
}

fn rig_for(tag: &str, base: &str) -> Rig {
    rig_with(tag, |c| c.router_url = base.to_string())
}

#[test]
fn the_router_side_is_pulled_into_three_feeds_with_since_from_the_last_reading() {
    let up = upstream(router_routes());
    let r = rig_for("pull-ok", &up.base);
    pull_router_side(&r.app, &agent());
    let f = json(&get(&r, "/api/feeds"));
    for name in ["router", "cloud", "health"] {
        assert_eq!(f["feeds"][name]["present"], json!(true), "{name}");
        assert!(f["feeds"][name]["error"].is_null(), "{name}");
    }
    let first = up.seen.lock().unwrap().clone();
    assert!(
        first
            .iter()
            .any(|l| l.contains(&format!("GET /routes?since={} ", NOW - 86400))),
        "{first:?}"
    );
    pull_router_side(&r.app, &agent());
    let at = ai_console::clock::parse_rfc3339("2026-11-17T08:00:00Z").unwrap();
    assert!(
        up.seen
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.contains(&format!("since={} ", at - 60)))
    );
}

#[test]
fn an_unreachable_router_is_an_error_state_for_each_pulled_feed() {
    let r = rig_for("pull-down", "http://127.0.0.1:1");
    pull_router_side(&r.app, &agent());
    let f = json(&get(&r, "/api/feeds"));
    for name in ["router", "cloud", "health"] {
        assert_eq!(f["feeds"][name]["present"], json!(false), "{name}");
        assert!(
            f["feeds"][name]["error"]
                .as_str()
                .is_some_and(|e| !e.is_empty()),
            "{name}"
        );
    }
}

#[test]
fn a_router_reply_that_fails_validation_is_kept_as_an_error() {
    let mut routes = router_routes();
    routes.insert(
        "/cloud".into(),
        (200, r#"{"providers":{},"models":{"x":null}}"#.into()),
    );
    routes.insert("/health".into(), (500, "{}".into()));
    let up = upstream(routes);
    let r = rig_for("pull-bad", &up.base);
    pull_router_side(&r.app, &agent());
    let f = json(&get(&r, "/api/feeds"));
    assert_eq!(f["feeds"]["router"]["present"], json!(true));
    assert!(
        f["feeds"]["cloud"]["error"]
            .as_str()
            .unwrap()
            .contains("models.x")
    );
    assert_eq!(f["feeds"]["cloud"]["present"], json!(false));
    assert!(f["feeds"]["health"]["error"].is_string());
}

fn ha_routes() -> Routes {
    let ids = fixture("ha-entities.json");
    let states = fixture("ha-states.json");
    HashMap::from([
        ("/ha/template".to_string(), (401, ids)),
        ("/ha/states".to_string(), (401, states)),
    ])
}

#[test]
fn the_harness_is_read_through_the_supervisor_proxy_with_the_token() {
    let up = upstream(ha_routes());
    let r = rig("pull-ha");
    let base = format!("{}/ha", up.base);
    pull_harness(&r.app, &agent(), &base, Some("tok"));
    let f = json(&get(&r, "/api/feeds"));
    assert_eq!(
        f["feeds"]["harness"]["data"]["agents"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let seen = up.seen.lock().unwrap().clone();
    assert!(
        seen.iter()
            .any(|l| l.starts_with("POST /ha/template") && l.ends_with("auth=true")),
        "{seen:?}"
    );
    assert!(
        seen.iter()
            .any(|l| l.starts_with("GET /ha/states") && l.ends_with("auth=true")),
        "{seen:?}"
    );
    // a rejected token is an error state and the credential never appears in it
    pull_harness(&r.app, &agent(), &base, Some("wrong"));
    let f = json(&get(&r, "/api/feeds"));
    let err = f["feeds"]["harness"]["error"].as_str().unwrap();
    assert!(!err.contains("wrong") && !err.contains("tok"), "{err}");
}

#[test]
fn no_token_is_an_error_state_that_names_the_option() {
    let r = rig("pull-ha-none");
    pull_harness(&r.app, &agent(), "http://127.0.0.1:1", None);
    let f = json(&get(&r, "/api/feeds"));
    assert!(
        f["feeds"]["harness"]["error"]
            .as_str()
            .unwrap()
            .contains("homeassistant_api")
    );
    assert_eq!(f["feeds"]["harness"]["present"], json!(false));
}

/// Under s6-overlay a CMD does not inherit the container environment; the token is read from s6's copy.
#[test]
fn the_supervisor_token_is_read_from_the_s6_environment() {
    use ai_console::pull::supervisor_token;
    assert!(
        std::env::var_os("SUPERVISOR_TOKEN").is_none(),
        "this test needs SUPERVISOR_TOKEN unset"
    );
    let dir = std::env::temp_dir().join(format!("ai-console-s6-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let d = dir.to_str().unwrap();
    assert_eq!(supervisor_token(d), None);
    std::fs::write(dir.join("SUPERVISOR_TOKEN"), "  \n").unwrap();
    assert_eq!(supervisor_token(d), None);
    std::fs::write(dir.join("SUPERVISOR_TOKEN"), "fixture-token\n").unwrap();
    assert_eq!(supervisor_token(d).as_deref(), Some("fixture-token"));
    std::fs::remove_dir_all(&dir).unwrap();
}
