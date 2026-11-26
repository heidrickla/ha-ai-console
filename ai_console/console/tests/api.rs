mod common;
use common::*;
use serde_json::json;

fn feed_file(r: &Rig, name: &str) -> std::path::PathBuf {
    r.dir.join("feeds").join(format!("{name}.json"))
}

#[test]
fn a_push_from_a_listed_source_is_stored() {
    let r = rig("stored");
    for name in ["quota", "meter", "routes"] {
        let reply = post(&r, name, &fixture(&format!("{name}.json")), workstation());
        assert_eq!(reply.status, 204, "{name}: {}", text(&reply));
        assert!(feed_file(&r, name).is_file(), "{name} file");
    }
}

#[test]
fn a_push_from_another_address_is_403_and_writes_nothing() {
    let r = rig("refused");
    let stranger = Some("192.0.2.99".parse().unwrap());
    for from in [stranger, None] {
        let reply = post(&r, "meter", &fixture("meter.json"), from);
        assert_eq!(reply.status, 403);
    }
    assert!(!feed_file(&r, "meter").exists());
    let feeds = json(&get(&r, "/api/feeds"));
    assert_eq!(feeds["feeds"]["meter"]["present"], json!(false));
}

#[test]
fn an_invalid_push_is_400_names_the_field_and_replaces_nothing() {
    let r = rig("invalid");
    assert_eq!(
        post(&r, "quota", &fixture("quota.json"), workstation()).status,
        204
    );
    let before = std::fs::read_to_string(feed_file(&r, "quota")).unwrap();
    let mut bad = fixture_json("quota.json");
    bad["windows"][0]["used_pct"] = json!("lots");
    let reply = post(&r, "quota", &bad.to_string(), workstation());
    assert_eq!(reply.status, 400);
    assert!(
        text(&reply).contains("windows[0].used_pct"),
        "{}",
        text(&reply)
    );
    assert_eq!(
        std::fs::read_to_string(feed_file(&r, "quota")).unwrap(),
        before
    );
    let feeds = json(&get(&r, "/api/feeds"));
    assert!(
        feeds["feeds"]["quota"]["error"]
            .as_str()
            .unwrap()
            .contains("windows[0].used_pct")
    );
    assert_eq!(post(&r, "quota", "not json", workstation()).status, 400);
}

#[test]
fn a_body_over_4_mib_is_413_and_an_unknown_feed_404() {
    let r = rig("limits");
    let big = vec![b' '; 4 * 1024 * 1024 + 1];
    let reply = r.app.handle("POST", "/api/feed/meter", workstation(), &big);
    assert_eq!(reply.status, 413);
    assert!(!feed_file(&r, "meter").exists());
    assert_eq!(post(&r, "other", "{}", workstation()).status, 404);
    assert_eq!(
        r.app
            .handle("GET", "/api/feed/meter", workstation(), b"")
            .status,
        405
    );
}

fn routes_body(at: i64, events: &[(&str, i64, bool)]) -> String {
    let evs: Vec<_> = events
        .iter()
        .map(|(id, t, ok)| {
            json!({"id": id, "t": ai_console::clock::format_rfc3339(*t), "caller": "delegate",
                   "route": "claude", "model": "claude-opus-5-5", "ok": ok})
        })
        .collect();
    json!({"v": 1, "at": ai_console::clock::format_rfc3339(at), "source": "workstation", "events": evs}).to_string()
}

fn ids(r: &Rig, feed: &str) -> Vec<String> {
    let f = json(&get(r, "/api/feeds"));
    let mut v: Vec<String> = f["feeds"][feed]["data"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect();
    v.sort();
    v
}

#[test]
fn routes_merge_by_id_and_drop_events_older_than_24_hours() {
    let r = rig("merge");
    let h = 3600;
    let first = routes_body(
        NOW,
        &[
            ("a", NOW - 2 * h, true),
            ("b", NOW - 25 * h, true),
            ("c", NOW - 1800, true),
        ],
    );
    assert_eq!(post(&r, "routes", &first, workstation()).status, 204);
    assert_eq!(
        ids(&r, "routes"),
        ["a", "c"],
        "the 25 hour old event is dropped"
    );
    // a repeat of a with a changed outcome replaces it; d is new
    let second = routes_body(NOW, &[("a", NOW - 2 * h, false), ("d", NOW - 60, true)]);
    assert_eq!(post(&r, "routes", &second, workstation()).status, 204);
    assert_eq!(ids(&r, "routes"), ["a", "c", "d"]);
    let f = json(&get(&r, "/api/feeds"));
    let a = f["feeds"]["routes"]["data"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "a")
        .unwrap()
        .clone();
    assert_eq!(a["ok"], json!(false));
    // time passes: a and c age out of the store at the next read
    r.clock
        .store(NOW + 23 * h, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        post(
            &r,
            "routes",
            &routes_body(NOW + 23 * h, &[("e", NOW + 23 * h, true)]),
            workstation()
        )
        .status,
        204
    );
    assert_eq!(ids(&r, "routes"), ["c", "d", "e"]);
}

#[test]
fn the_merged_routes_survive_a_restart() {
    let r = rig("restart");
    assert_eq!(
        post(&r, "routes", &fixture("routes.json"), workstation()).status,
        204
    );
    let again = build(r.dir.clone(), |_| {});
    let f = json(&get(&again, "/api/feeds"));
    assert_eq!(
        f["feeds"]["routes"]["data"]["events"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn api_feeds_reports_each_feed_with_its_age_and_last_error() {
    let r = rig("ages");
    let f = json(&get(&r, "/api/feeds"));
    for name in [
        "quota", "meter", "routes", "router", "cloud", "health", "harness",
    ] {
        assert_eq!(f["feeds"][name]["present"], json!(false), "{name}");
        assert!(f["feeds"][name]["age_s"].is_null(), "{name}");
    }
    assert_eq!(
        post(&r, "meter", &fixture("meter.json"), workstation()).status,
        204
    );
    let f = json(&get(&r, "/api/feeds"));
    assert_eq!(f["feeds"]["meter"]["present"], json!(true));
    assert_eq!(f["feeds"]["meter"]["age_s"], json!(30));
    assert_eq!(
        f["feeds"]["meter"]["data"]["cycle"]["budget_usd"],
        json!(169.53)
    );
    r.clock.store(NOW + 90, std::sync::atomic::Ordering::SeqCst);
    let f = json(&get(&r, "/api/feeds"));
    assert_eq!(f["feeds"]["meter"]["age_s"], json!(120));
    assert!(
        f["now"]
            .as_str()
            .unwrap()
            .starts_with("2026-11-17T08:02:00")
    );
}

#[test]
fn pulled_feeds_are_stored_and_an_unreachable_router_is_a_state() {
    let r = rig("pulled");
    r.app
        .ingest_pulled("router", &fixture("router.json"))
        .unwrap();
    r.app
        .ingest_pulled("cloud", &fixture("cloud.json"))
        .unwrap();
    r.app
        .ingest_pulled("health", &fixture("health.json"))
        .unwrap();
    let f = json(&get(&r, "/api/feeds"));
    assert_eq!(f["feeds"]["router"]["present"], json!(true));
    assert_eq!(
        f["feeds"]["router"]["data"]["events"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        f["feeds"]["cloud"]["data"]["providers"]["openrouter"]["requests_today"],
        json!(38)
    );
    assert_eq!(f["feeds"]["health"]["age_s"], json!(0));
    r.app.pull_failed("router", "connection refused");
    let f = json(&get(&r, "/api/feeds"));
    assert_eq!(f["feeds"]["router"]["error"], json!("connection refused"));
    assert_eq!(
        f["feeds"]["router"]["present"],
        json!(true),
        "the last good reading stays"
    );
    let bad = r
        .app
        .ingest_pulled("cloud", "{\"enabled\":[]}")
        .unwrap_err();
    assert!(bad.contains("providers"), "{bad}");
}

#[test]
fn the_router_poll_is_fast_only_while_a_page_polls() {
    let r = rig("cadence");
    assert_eq!(r.app.poll_interval_s(), 60);
    get(&r, "/api/feeds");
    assert_eq!(r.app.poll_interval_s(), 15);
    r.mono.fetch_add(121, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(r.app.poll_interval_s(), 60);
}

#[test]
fn a_backward_step_of_the_wall_clock_does_not_hold_the_fast_poll() {
    let r = rig("cadence-wall");
    get(&r, "/api/feeds");
    r.mono.fetch_add(121, std::sync::atomic::Ordering::SeqCst);
    r.clock
        .store(NOW - 3600, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(r.app.poll_interval_s(), 60);
}

#[test]
fn the_router_since_is_the_window_start_until_a_reading_arrives_then_its_own_at() {
    let r = rig("since");
    assert_eq!(r.app.router_since(), NOW - 86400);
    r.app
        .ingest_pulled("router", &fixture("router.json"))
        .unwrap();
    let at = ai_console::clock::parse_rfc3339("2026-11-17T08:00:00Z").unwrap();
    assert_eq!(r.app.router_since(), at - 60);
}

#[test]
fn the_harness_feed_is_stored_from_the_tracker_entities() {
    let r = rig("harness");
    let ids: Vec<String> = serde_json::from_value(fixture_json("ha-entities.json")).unwrap();
    let states: Vec<serde_json::Value> =
        serde_json::from_value(fixture_json("ha-states.json")).unwrap();
    r.app
        .ingest_harness(ai_console::harness::harness_from(&ids, &states));
    let f = json(&get(&r, "/api/feeds"));
    assert_eq!(f["feeds"]["harness"]["present"], json!(true));
    assert_eq!(
        f["feeds"]["harness"]["data"]["agents"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

fn with_at(doc: &str, at: i64) -> String {
    let mut v: serde_json::Value = serde_json::from_str(doc).unwrap();
    v["at"] = json!(ai_console::clock::format_rfc3339(at));
    v.to_string()
}

#[test]
fn a_feed_dated_more_than_five_minutes_ahead_is_refused_naming_the_field() {
    let r = rig("future-at");
    let reply = post(
        &r,
        "meter",
        &with_at(&fixture("meter.json"), NOW + 600),
        workstation(),
    );
    assert_eq!(reply.status, 400);
    assert!(
        text(&reply).contains("`at`") && text(&reply).contains("clock"),
        "{}",
        text(&reply)
    );
    assert!(!r.dir.join("feeds/meter.json").exists());
}

#[test]
fn a_feed_slightly_ahead_is_kept_and_shown_as_clock_skew() {
    let r = rig("skew");
    assert_eq!(
        post(
            &r,
            "meter",
            &with_at(&fixture("meter.json"), NOW + 120),
            workstation()
        )
        .status,
        204
    );
    let f = json(&get(&r, "/api/feeds"));
    assert_eq!(f["feeds"]["meter"]["skew_s"], json!(120));
    assert_eq!(f["feeds"]["meter"]["age_s"], json!(0));
    assert!(f["feeds"]["quota"]["skew_s"].is_null());
}

#[test]
fn an_event_dated_ahead_is_refused_and_cannot_move_the_router_pull() {
    let r = rig("future-event");
    let ahead = routes_body(NOW, &[("f", NOW + 86400, true)]);
    let reply = post(&r, "routes", &ahead, workstation());
    assert_eq!(reply.status, 400);
    assert!(text(&reply).contains("events[0].t"), "{}", text(&reply));
    let mut doc: serde_json::Value = serde_json::from_str(&fixture("router.json")).unwrap();
    doc["events"][0]["t"] = json!(ai_console::clock::format_rfc3339(NOW + 86400));
    let err = r.app.ingest_pulled("router", &doc.to_string()).unwrap_err();
    assert!(err.contains("events[0].t"), "{err}");
    assert_eq!(r.app.router_since(), NOW - 86400);
    let f = json(&get(&r, "/api/feeds"));
    assert!(
        f["feeds"]["router"]["error"]
            .as_str()
            .unwrap()
            .contains("events[0].t")
    );
}

#[test]
fn a_stored_event_list_is_capped_and_the_page_gets_the_newest_few() {
    let r = rig_with("cap", |c| {
        c.max_events = 50;
        c.page_events = 10;
    });
    let evs: Vec<(String, i64, bool)> = (0..120)
        .map(|i| (format!("e{i:03}"), NOW - 1000 + i, true))
        .collect();
    let refs: Vec<(&str, i64, bool)> = evs.iter().map(|(a, b, c)| (a.as_str(), *b, *c)).collect();
    assert_eq!(
        post(&r, "routes", &routes_body(NOW, &refs), workstation()).status,
        204
    );
    let stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(r.dir.join("feeds/routes.json")).unwrap())
            .unwrap();
    assert_eq!(stored["events"].as_array().unwrap().len(), 50);
    assert_eq!(
        stored["events"][49]["id"],
        json!("e119"),
        "the newest are kept"
    );
    let f = json(&get(&r, "/api/feeds"));
    assert_eq!(
        f["feeds"]["routes"]["data"]["events"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    assert_eq!(f["feeds"]["routes"]["events_total"], json!(50));
}

#[test]
fn a_pulled_feed_that_fails_validation_is_an_error_state_and_keeps_the_last_good_reading() {
    let r = rig("pulled-bad");
    r.app
        .ingest_pulled("cloud", &fixture("cloud.json"))
        .unwrap();
    for bad in [
        r#"{"providers":{},"models":{"x":null}}"#,
        r#"{"providers":{},"enabled":"a"}"#,
        r#"{"providers":{"p":{"request_limit":"lots"}}}"#,
    ] {
        assert!(r.app.ingest_pulled("cloud", bad).is_err(), "{bad}");
    }
    let f = json(&get(&r, "/api/feeds"));
    assert!(f["feeds"]["cloud"]["error"].is_string());
    assert_eq!(f["feeds"]["cloud"]["present"], json!(true));
    assert_eq!(
        f["feeds"]["cloud"]["data"]["providers"]["openrouter"]["requests_today"],
        json!(38)
    );
}

#[test]
fn files_that_cannot_be_read_back_are_discarded_at_start_and_named() {
    let dir = dir_for("discard");
    std::fs::create_dir_all(dir.join("feeds")).unwrap();
    std::fs::write(dir.join("feeds/meter.json"), b"").unwrap();
    std::fs::write(dir.join("feeds/routes.json"), b"{not json").unwrap();
    std::fs::write(
        dir.join("feeds/quota.json"),
        br#"{"v":1,"at":"2026-11-17T08:00:00Z","windows":[{"plan":""}]}"#,
    )
    .unwrap();
    std::fs::write(dir.join("feeds/health.json"), fixture("health.json")).unwrap();
    let r = build(dir, |_| {});
    let notes = r.app.discarded().join("\n");
    for name in ["meter", "routes", "quota"] {
        assert!(notes.contains(&format!("{name}.json")), "{notes}");
    }
    assert!(!notes.contains("health.json"));
    let f = json(&get(&r, "/api/feeds"));
    assert_eq!(f["feeds"]["meter"]["present"], json!(false));
    assert_eq!(f["feeds"]["health"]["present"], json!(true));
}

#[test]
fn a_write_leaves_no_temporary_file_and_replaces_the_feed_whole() {
    let r = rig("atomic");
    for _ in 0..3 {
        assert_eq!(
            post(&r, "meter", &fixture("meter.json"), workstation()).status,
            204
        );
    }
    let names: Vec<String> = std::fs::read_dir(r.dir.join("feeds"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["meter.json"]);
}

#[test]
fn a_stored_event_file_is_capped_and_aged_at_load_too() {
    let dir = dir_for("load-cap");
    std::fs::create_dir_all(dir.join("feeds")).unwrap();
    let mut evs: Vec<serde_json::Value> = (0..120)
        .map(|i| json!({"id": format!("e{i:03}"), "t": ai_console::clock::format_rfc3339(NOW - 1000 + i), "caller": "delegate", "route": "claude", "ok": true}))
        .collect();
    evs.push(json!({"id": "old", "t": ai_console::clock::format_rfc3339(NOW - 30 * 3600), "caller": "delegate", "route": "claude", "ok": true}));
    let doc = json!({"v": 1, "at": ai_console::clock::format_rfc3339(NOW), "events": evs});
    std::fs::write(dir.join("feeds/routes.json"), doc.to_string()).unwrap();
    let r = build(dir, |c| {
        c.max_events = 50;
        c.page_events = 1000;
    });
    let f = json(&get(&r, "/api/feeds"));
    let ids: Vec<&str> = f["feeds"]["routes"]["data"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 50);
    assert_eq!(ids[0], "e070");
    assert_eq!(ids[49], "e119");
}

fn broken_feeds_dir(tag: &str) -> Rig {
    let dir = dir_for(tag);
    std::fs::write(dir.join("feeds"), b"a file where the directory should be").unwrap();
    build(dir, |_| {})
}

#[test]
fn a_push_that_cannot_be_saved_is_500_and_is_not_kept() {
    let r = broken_feeds_dir("nosave");
    let reply = post(&r, "meter", &fixture("meter.json"), workstation());
    assert_eq!(reply.status, 500);
    assert!(text(&reply).contains("cannot save"), "{}", text(&reply));
    let f = json(&get(&r, "/api/feeds"));
    assert_eq!(f["feeds"]["meter"]["present"], json!(false));
    assert!(
        f["feeds"]["meter"]["error"]
            .as_str()
            .unwrap()
            .contains("cannot save")
    );
}

#[test]
fn a_failed_snapshot_leaves_the_last_good_file_and_state() {
    let r = rig("keep-good");
    assert_eq!(
        post(&r, "meter", &fixture("meter.json"), workstation()).status,
        204
    );
    let good = std::fs::read_to_string(r.dir.join("feeds/meter.json")).unwrap();
    // a directory where the temporary file goes makes the next write fail
    std::fs::create_dir(r.dir.join("feeds/meter.json.tmp")).unwrap();
    let mut changed = fixture_json("meter.json");
    changed["cycle"]["spent_usd"] = json!(99.0);
    assert_eq!(
        post(&r, "meter", &changed.to_string(), workstation()).status,
        500
    );
    assert_eq!(
        std::fs::read_to_string(r.dir.join("feeds/meter.json")).unwrap(),
        good
    );
    let f = json(&get(&r, "/api/feeds"));
    assert_eq!(
        f["feeds"]["meter"]["data"]["cycle"]["spent_usd"],
        json!(20.43)
    );
}

#[test]
fn a_pulled_feed_that_cannot_be_saved_still_shows_and_says_so() {
    let r = broken_feeds_dir("nosave-pulled");
    r.app
        .ingest_pulled("cloud", &fixture("cloud.json"))
        .unwrap();
    let f = json(&get(&r, "/api/feeds"));
    assert_eq!(f["feeds"]["cloud"]["present"], json!(true));
    assert!(
        f["feeds"]["cloud"]["error"]
            .as_str()
            .unwrap()
            .contains("cannot save")
    );
}
