//! Feed validation: the first problem found, with the field named. A field the producer could not
//! read is absent or null, so optional fields accept null and are type-checked when present.

use serde_json::Value;

use crate::clock::parse_rfc3339;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Quota,
    Meter,
    Routes,
    Router,
    Cloud,
    Health,
}

type R = Result<(), String>;

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

fn bad(path: &str, key: &str, why: &str) -> String {
    format!("field `{}`: {why}", join(path, key))
}

fn present<'a>(v: &'a Value, path: &str, key: &str) -> Result<Option<&'a Value>, String> {
    match v {
        Value::Object(m) => Ok(m.get(key).filter(|x| !x.is_null())),
        _ => Err(format!("field `{path}`: expected an object")),
    }
}

fn need<'a>(v: &'a Value, path: &str, key: &str) -> Result<&'a Value, String> {
    present(v, path, key)?.ok_or_else(|| bad(path, key, "missing"))
}

fn num(v: &Value, path: &str, key: &str, required: bool) -> R {
    match present(v, path, key)? {
        Some(x) if x.is_number() => Ok(()),
        Some(_) => Err(bad(path, key, "expected a number")),
        None if required => Err(bad(path, key, "missing")),
        None => Ok(()),
    }
}

fn string(v: &Value, path: &str, key: &str, required: bool) -> R {
    match present(v, path, key)? {
        Some(x) if x.is_string() => Ok(()),
        Some(_) => Err(bad(path, key, "expected a string")),
        None if required => Err(bad(path, key, "missing")),
        None => Ok(()),
    }
}

fn name(v: &Value, path: &str, key: &str, required: bool) -> R {
    match present(v, path, key)? {
        Some(Value::String(s)) if !s.is_empty() && s.len() <= 300 => Ok(()),
        Some(Value::String(_)) => Err(bad(
            path,
            key,
            "expected a non-empty string of at most 300 bytes",
        )),
        Some(_) => Err(bad(path, key, "expected a string")),
        None if required => Err(bad(path, key, "missing")),
        None => Ok(()),
    }
}

fn positive(v: &Value, path: &str, key: &str) -> R {
    match present(v, path, key)?.and_then(Value::as_f64) {
        Some(n) if n > 0.0 => Ok(()),
        _ => Err(bad(path, key, "expected a number above 0")),
    }
}

fn boolean(v: &Value, path: &str, key: &str) -> R {
    match present(v, path, key)? {
        None | Some(Value::Bool(_)) => Ok(()),
        Some(_) => Err(bad(path, key, "expected true or false")),
    }
}

fn date(v: &Value, path: &str, key: &str) -> R {
    let ok = present(v, path, key)?
        .and_then(Value::as_str)
        .is_some_and(|d| {
            let b = d.as_bytes();
            b.len() == 10
                && b[4] == b'-'
                && b[7] == b'-'
                && d.bytes()
                    .enumerate()
                    .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
        });
    if ok {
        Ok(())
    } else {
        Err(bad(path, key, "expected a date as YYYY-MM-DD"))
    }
}

fn time(v: &Value, path: &str, key: &str, required: bool) -> R {
    match present(v, path, key)? {
        Some(Value::String(s)) if parse_rfc3339(s).is_some() => Ok(()),
        Some(_) => Err(bad(path, key, "expected an RFC 3339 time")),
        None if required => Err(bad(path, key, "missing")),
        None => Ok(()),
    }
}

fn object<'a>(v: &'a Value, path: &str, key: &str) -> Result<&'a Value, String> {
    let x = need(v, path, key)?;
    if x.is_object() {
        Ok(x)
    } else {
        Err(bad(path, key, "expected an object"))
    }
}

fn rows<'a>(v: &'a Value, path: &str, key: &str, required: bool) -> Result<&'a [Value], String> {
    match present(v, path, key)? {
        Some(Value::Array(a)) => Ok(a),
        Some(_) => Err(bad(path, key, "expected an array")),
        None if required => Err(bad(path, key, "missing")),
        None => Ok(&[]),
    }
}

fn each(v: &Value, path: &str, key: &str, required: bool, check: impl Fn(&Value, &str) -> R) -> R {
    for (i, row) in rows(v, path, key, required)?.iter().enumerate() {
        check(row, &format!("{}[{i}]", join(path, key)))?;
    }
    Ok(())
}

fn envelope(v: &Value) -> R {
    if need(v, "", "v")?.as_i64() != Some(1) {
        return Err(bad("", "v", "expected 1"));
    }
    time(v, "", "at", true)
}

fn nums(v: &Value, path: &str, required: &[&str], optional: &[&str]) -> R {
    for k in required {
        num(v, path, k, true)?;
    }
    for k in optional {
        num(v, path, k, false)?;
    }
    Ok(())
}

fn stats(v: &Value, path: &str) -> R {
    nums(
        v,
        path,
        &[
            "requests",
            "ok",
            "failed",
            "refused",
            "input_tokens",
            "output_tokens",
            "cache_read",
            "cache_write",
            "cost_usd",
        ],
        &["p50_latency_ms"],
    )
}

fn meter(v: &Value) -> R {
    let cycle = object(v, "", "cycle")?;
    time(cycle, "cycle", "start", true)?;
    time(cycle, "cycle", "end", true)?;
    nums(
        cycle,
        "cycle",
        &["budget_usd", "spent_usd", "reserved_usd"],
        &[],
    )?;
    let tiles = object(v, "", "tiles")?;
    need(tiles, "tiles", "window_h")?;
    positive(tiles, "tiles", "window_h")?;
    stats(object(tiles, "tiles", "now")?, "tiles.now")?;
    stats(object(tiles, "tiles", "prev")?, "tiles.prev")?;
    each(tiles, "tiles", "spark", true, |r, p| {
        time(r, p, "t", true)?;
        nums(r, p, &["requests", "tokens", "cost_usd"], &[])
    })?;
    each(v, "", "hourly", true, |r, p| {
        time(r, p, "t", true)?;
        name(r, p, "model", true)?;
        nums(
            r,
            p,
            &[
                "requests",
                "input_tokens",
                "output_tokens",
                "cache_read",
                "cache_write",
                "cost_usd",
            ],
            &[],
        )
    })?;
    each(v, "", "by_model", true, |r, p| {
        name(r, p, "model", true)?;
        nums(r, p, &["requests", "tokens", "cost_usd", "share"], &[])
    })?;
    each(v, "", "records", true, |r, p| {
        time(r, p, "t", true)?;
        time(r, p, "t_end", false)?;
        name(r, p, "model", true)?;
        name(r, p, "status", true)?;
        string(r, p, "label", false)?;
        nums(
            r,
            p,
            &["id"],
            &[
                "http_status",
                "input_tokens",
                "output_tokens",
                "cache_read",
                "cache_write",
                "cost_usd",
            ],
        )
    })?;
    each(v, "", "daily", true, |r, p| {
        date(r, p, "date")?;
        nums(r, p, &["requests", "tokens", "cost_usd"], &[])
    })
}

fn quota(v: &Value) -> R {
    string(v, "", "source", false)?;
    each(v, "", "windows", true, |r, p| {
        for k in ["plan", "window", "scope"] {
            name(r, p, k, true)?;
        }
        nums(r, p, &["used_pct"], &[])?;
        need(r, p, "length_s")?;
        positive(r, p, "length_s")?;
        time(r, p, "resets_at", true)?;
        time(r, p, "read_at", false)
    })?;
    each(v, "", "errors", false, |r, p| {
        name(r, p, "plan", true)?;
        string(r, p, "error", true)
    })
}

fn events(v: &Value) -> R {
    each(v, "", "events", true, |e, p| {
        name(e, p, "id", true)?;
        time(e, p, "t", true)?;
        name(e, p, "caller", true)?;
        name(e, p, "route", true)?;
        for k in ["model", "requested", "error"] {
            string(e, p, k, false)?;
        }
        nums(
            e,
            p,
            &[],
            &[
                "status",
                "latency_ms",
                "tool_calls",
                "input_tokens",
                "output_tokens",
            ],
        )?;
        boolean(e, p, "ok")
    })
}

fn quotas(v: &Value, path: &str) -> R {
    each(v, path, "quotas", false, |q, p| {
        for k in ["limits", "remaining"] {
            if let Some(m) = present(q, p, k)? {
                let Some(m) = m.as_object() else {
                    return Err(bad(p, k, "expected an object"));
                };
                if let Some((key, _)) = m.iter().find(|(_, x)| !x.is_number()) {
                    return Err(bad(&join(p, k), key, "expected a number"));
                }
            }
        }
        Ok(())
    })
}

fn cloud(v: &Value) -> R {
    for (i, n) in rows(v, "", "enabled", false)?.iter().enumerate() {
        if !n.as_str().is_some_and(|s| !s.is_empty()) {
            return Err(format!("field `enabled[{i}]`: expected a non-empty string"));
        }
    }
    let providers = object(v, "", "providers")?;
    for (key, row) in providers.as_object().into_iter().flatten() {
        let p = join("providers", key);
        if !row.is_object() {
            return Err(format!("field `{p}`: expected an object"));
        }
        nums(
            row,
            &p,
            &[],
            &["requests_today", "tokens_today", "backoff_until"],
        )?;
        string(row, &p, "backoff_reason", false)?;
        boolean(row, &p, "credential_refused")?;
        match present(row, &p, "request_limit")? {
            None => {}
            Some(x) if x.is_number() || x.as_str() == Some("per model") => {}
            Some(_) => {
                return Err(bad(
                    &p,
                    "request_limit",
                    "expected a number or \"per model\"",
                ));
            }
        }
        quotas(row, &p)?;
    }
    if let Some(models) = present(v, "", "models")? {
        let Some(models) = models.as_object() else {
            return Err(bad("", "models", "expected an object"));
        };
        for (key, row) in models {
            let p = join("models", key);
            if !row.is_object() {
                return Err(format!("field `{p}`: expected an object"));
            }
            nums(row, &p, &[], &["backoff_until"])?;
            string(row, &p, "backoff_why", false)?;
            string(row, &p, "reason", false)?;
            boolean(row, &p, "routable")?;
        }
    }
    Ok(())
}

fn health(v: &Value) -> R {
    each(v, "", "hosts", true, |h, p| {
        name(h, p, "name", true)?;
        // The router state: voice and background are calls in flight,
        // committed and slots_committed are context tokens, capacity is the context size (n_ctx),
        // ceded names the swap-group peer holding the card. null means not known.
        for k in ["ready", "gate_up", "swap_up"] {
            boolean(h, p, k)?;
        }
        string(h, p, "ceded", false)?;
        nums(
            h,
            p,
            &[],
            &[
                "voice",
                "background",
                "committed",
                "capacity",
                "slots_busy",
                "slots_committed",
            ],
        )
    })?;
    boolean(v, "", "voice_live")?;
    string(v, "", "voice_host", false)?;
    nums(v, "", &[], &["waiting"])?;
    if let Some(c) = present(v, "", "cloud")? {
        if !c.is_object() {
            return Err(bad("", "cloud", "expected an object"));
        }
        nums(
            c,
            "cloud",
            &[],
            &["providers_enabled", "requests_today", "models_backing_off"],
        )?;
    }
    Ok(())
}

pub fn validate(kind: Kind, v: &Value) -> Result<(), String> {
    if !v.is_object() {
        return Err("field `(body)`: expected a JSON object".into());
    }
    match kind {
        Kind::Quota => envelope(v).and_then(|_| quota(v)),
        Kind::Meter => envelope(v).and_then(|_| meter(v)),
        Kind::Routes | Kind::Router => envelope(v).and_then(|_| events(v)),
        Kind::Cloud => cloud(v),
        Kind::Health => health(v),
    }
}

/// How far ahead of the add-on's clock a producer's timestamp may be before its feed is refused.
pub const SKEW_S: i64 = 300;

/// Refuse a feed whose `at`, or an event's `t`, is ahead of `now` by more than SKEW_S.
pub fn validate_fresh(kind: Kind, v: &Value, now: i64) -> Result<(), String> {
    let ahead = |path: String, iso: Option<&str>| -> R {
        match iso.and_then(parse_rfc3339) {
            Some(t) if t > now + SKEW_S => Err(format!(
                "field `{path}`: {}s ahead of this clock, more than {SKEW_S}s of clock skew",
                t - now
            )),
            _ => Ok(()),
        }
    };
    if kind != Kind::Cloud && kind != Kind::Health {
        ahead("at".into(), v["at"].as_str())?;
    }
    if kind == Kind::Routes || kind == Kind::Router {
        for (i, e) in v["events"].as_array().into_iter().flatten().enumerate() {
            ahead(format!("events[{i}].t"), e["t"].as_str())?;
        }
    }
    Ok(())
}
