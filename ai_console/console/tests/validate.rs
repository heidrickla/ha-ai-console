mod common;
use ai_console::validate::{Kind, validate};
use common::fixture_json;
use serde_json::{Value, json};

fn walk<'a>(v: &'a mut Value, path: &str) -> &'a mut Value {
    let mut cur = v;
    for seg in path.split('.') {
        cur = match seg.parse::<usize>() {
            Ok(i) => &mut cur[i],
            Err(_) => &mut cur[seg],
        };
    }
    cur
}

fn without(mut v: Value, path: &str) -> Value {
    match path.rsplit_once('.') {
        Some((parent, last)) => walk(&mut v, parent).as_object_mut().unwrap().remove(last),
        None => v.as_object_mut().unwrap().remove(path),
    };
    v
}

fn with(mut v: Value, path: &str, new: Value) -> Value {
    *walk(&mut v, path) = new;
    v
}

const FIXTURES: [(&str, Kind); 6] = [
    ("quota.json", Kind::Quota),
    ("meter.json", Kind::Meter),
    ("routes.json", Kind::Routes),
    ("router.json", Kind::Router),
    ("cloud.json", Kind::Cloud),
    ("health.json", Kind::Health),
];

/// The router's own answers, captured from a live router on 2026-11-17 (models trimmed to 25).
#[test]
fn the_live_router_answers_are_accepted() {
    assert_eq!(
        validate(Kind::Health, &fixture_json("live-health.json")),
        Ok(())
    );
    assert_eq!(
        validate(Kind::Cloud, &fixture_json("live-cloud.json")),
        Ok(())
    );
}

#[test]
fn each_fixture_is_accepted() {
    for (name, kind) in FIXTURES {
        assert_eq!(validate(kind, &fixture_json(name)), Ok(()), "{name}");
    }
}

fn names_field(kind: Kind, v: &Value, field: &str) {
    let e = validate(kind, v).expect_err(field);
    assert!(e.contains(field), "{e:?} should name {field}");
}

#[test]
fn a_missing_field_is_named() {
    let m = fixture_json("meter.json");
    names_field(
        Kind::Meter,
        &without(m.clone(), "cycle.budget_usd"),
        "cycle.budget_usd",
    );
    names_field(Kind::Meter, &without(m.clone(), "tiles.now"), "tiles.now");
    names_field(Kind::Meter, &without(m, "at"), "at");
    let q = fixture_json("quota.json");
    names_field(
        Kind::Quota,
        &without(q, "windows.0.plan"),
        "windows[0].plan",
    );
    let r = fixture_json("routes.json");
    names_field(Kind::Routes, &without(r, "events.1.id"), "events[1].id");
    let h = fixture_json("health.json");
    names_field(Kind::Health, &without(h, "hosts.0.name"), "hosts[0].name");
    let c = fixture_json("cloud.json");
    names_field(Kind::Cloud, &without(c, "providers"), "providers");
}

#[test]
fn a_wrong_type_is_named() {
    let m = fixture_json("meter.json");
    names_field(
        Kind::Meter,
        &with(m.clone(), "tiles.now.requests", json!("12")),
        "tiles.now.requests",
    );
    names_field(Kind::Meter, &with(m.clone(), "v", json!(2)), "v");
    names_field(
        Kind::Meter,
        &with(m, "records.0.t", json!("yesterday")),
        "records[0].t",
    );
    let q = fixture_json("quota.json");
    names_field(
        Kind::Quota,
        &with(q, "windows.2.used_pct", json!("18")),
        "windows[2].used_pct",
    );
    let r = fixture_json("routes.json");
    names_field(
        Kind::Routes,
        &with(r, "events.0.ok", json!("yes")),
        "events[0].ok",
    );
    let router = fixture_json("router.json");
    names_field(
        Kind::Router,
        &with(router, "events.1.status", json!("429")),
        "events[1].status",
    );
}

#[test]
fn a_null_ok_is_accepted() {
    let r = with(fixture_json("routes.json"), "events.0.ok", Value::Null);
    assert_eq!(validate(Kind::Routes, &r), Ok(()));
}

fn rejects(kind: Kind, v: &Value, field: &str) {
    let e = validate(kind, v).expect_err(&format!("{field} in {v}"));
    assert!(e.contains(field), "{e:?} should name {field}");
}

#[test]
fn every_field_the_page_reads_is_checked_in_cloud() {
    let ok = fixture_json("cloud.json");
    let mut v = ok.clone();
    v["models"]["x"] = Value::Null;
    rejects(Kind::Cloud, &v, "models.x");
    rejects(
        Kind::Cloud,
        &with(ok.clone(), "enabled", json!("a")),
        "enabled",
    );
    rejects(
        Kind::Cloud,
        &with(ok.clone(), "enabled", json!([1])),
        "enabled[0]",
    );
    rejects(
        Kind::Cloud,
        &with(
            ok.clone(),
            "providers.openrouter.request_limit",
            json!("lots"),
        ),
        "providers.openrouter.request_limit",
    );
    rejects(
        Kind::Cloud,
        &with(
            ok.clone(),
            "providers.openrouter.backoff_until",
            json!("soon"),
        ),
        "providers.openrouter.backoff_until",
    );
    rejects(
        Kind::Cloud,
        &with(ok.clone(), "providers.openrouter.quotas", json!({})),
        "providers.openrouter.quotas",
    );
    rejects(
        Kind::Cloud,
        &with(
            ok.clone(),
            "providers.openrouter.quotas.0.limits",
            json!([]),
        ),
        "providers.openrouter.quotas[0].limits",
    );
    rejects(
        Kind::Cloud,
        &with(
            ok.clone(),
            "providers.openrouter.credential_refused",
            json!("no"),
        ),
        "providers.openrouter.credential_refused",
    );
    let mut v = ok;
    v["models"]["zai/glm-4.7-flash"]["backoff_why"] = json!(3);
    rejects(Kind::Cloud, &v, "models.zai/glm-4.7-flash.backoff_why");
}

#[test]
fn every_field_the_page_reads_is_checked_in_health() {
    let ok = fixture_json("health.json");
    rejects(
        Kind::Health,
        &with(ok.clone(), "hosts.0.name", json!("")),
        "hosts[0].name",
    );
    rejects(
        Kind::Health,
        &with(ok.clone(), "hosts.0.ready", json!("yes")),
        "hosts[0].ready",
    );
    rejects(
        Kind::Health,
        &with(ok.clone(), "hosts.0.capacity", json!("4")),
        "hosts[0].capacity",
    );
    rejects(
        Kind::Health,
        &with(ok.clone(), "hosts.0.voice", json!(true)),
        "hosts[0].voice",
    );
    rejects(
        Kind::Health,
        &with(ok.clone(), "hosts.1.ceded", json!(true)),
        "hosts[1].ceded",
    );
    rejects(
        Kind::Health,
        &with(ok.clone(), "voice_live", json!(1)),
        "voice_live",
    );
    rejects(
        Kind::Health,
        &with(ok.clone(), "waiting", json!("0")),
        "waiting",
    );
    rejects(Kind::Health, &with(ok, "hosts", json!({})), "hosts");
}

#[test]
fn every_field_the_page_reads_is_checked_in_quota_and_meter() {
    let q = fixture_json("quota.json");
    rejects(
        Kind::Quota,
        &with(q.clone(), "windows.0.plan", json!("")),
        "windows[0].plan",
    );
    rejects(
        Kind::Quota,
        &with(q.clone(), "windows.0.length_s", json!(0)),
        "windows[0].length_s",
    );
    rejects(
        Kind::Quota,
        &with(q.clone(), "errors", json!([{"plan": "codex"}])),
        "errors[0].error",
    );
    rejects(
        Kind::Quota,
        &with(q, "windows.0.window", json!(5)),
        "windows[0].window",
    );
    let m = fixture_json("meter.json");
    rejects(
        Kind::Meter,
        &with(m.clone(), "records.0.model", json!("")),
        "records[0].model",
    );
    rejects(
        Kind::Meter,
        &with(m.clone(), "daily.0.date", json!("17 Nov")),
        "daily[0].date",
    );
    rejects(
        Kind::Meter,
        &with(m.clone(), "tiles.window_h", json!(0)),
        "tiles.window_h",
    );
    rejects(
        Kind::Meter,
        &with(m.clone(), "hourly.0.model", json!(7)),
        "hourly[0].model",
    );
    rejects(
        Kind::Meter,
        &with(m, "by_model.0.share", json!("0.6")),
        "by_model[0].share",
    );
}

#[test]
fn an_event_needs_non_empty_names() {
    let r = fixture_json("routes.json");
    rejects(
        Kind::Routes,
        &with(r.clone(), "events.0.route", json!("")),
        "events[0].route",
    );
    rejects(
        Kind::Routes,
        &with(r.clone(), "events.0.caller", json!("")),
        "events[0].caller",
    );
    rejects(
        Kind::Routes,
        &with(r, "events.0.id", json!("")),
        "events[0].id",
    );
}

#[test]
fn a_future_time_is_refused_by_the_freshness_check_only() {
    use ai_console::validate::validate_fresh;
    let now = common::NOW;
    let m = fixture_json("meter.json");
    assert_eq!(validate_fresh(Kind::Meter, &m, now), Ok(()));
    let ahead = with(m, "at", json!("2026-11-17T08:30:00Z"));
    assert_eq!(validate(Kind::Meter, &ahead), Ok(()));
    assert!(
        validate_fresh(Kind::Meter, &ahead, now)
            .unwrap_err()
            .contains("`at`")
    );
}
