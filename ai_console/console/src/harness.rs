//! The tracker's Home Assistant entities reduced to one row per agent. The ids come from the
//! tracker's integration, so an entity of another integration never matches by name.

use std::collections::BTreeMap;

use serde_json::{Value, json};

const SUFFIXES: [(&str, &str, &str); 3] = [
    ("sensor.", "_window_success_rate", "window_success_rate"),
    ("sensor.", "_success_rate", "success_rate"),
    ("binary_sensor.", "_harness_regressed", "regressed"),
];

fn reading(state: &str) -> Option<&str> {
    (!matches!(state, "unavailable" | "unknown" | "")).then_some(state)
}

pub fn harness_from(ids: &[String], states: &[Value]) -> Value {
    let mut agents: BTreeMap<String, Value> = BTreeMap::new();
    for s in states {
        let Some(id) = s["entity_id"].as_str() else {
            continue;
        };
        if !ids.iter().any(|i| i == id) {
            continue;
        }
        let state = s["state"].as_str().unwrap_or("unavailable");
        for (domain, suffix, field) in SUFFIXES {
            let Some(agent) = id.strip_prefix(domain).and_then(|r| r.strip_suffix(suffix)) else {
                continue;
            };
            let row = agents.entry(agent.to_string()).or_insert_with(|| {
                json!({"agent": agent, "regressed": "unavailable", "success_rate": null, "window_success_rate": null})
            });
            row[field] = if field == "regressed" {
                json!(
                    reading(state)
                        .filter(|v| *v == "on" || *v == "off")
                        .unwrap_or("unavailable")
                )
            } else {
                reading(state)
                    .and_then(|v| v.parse::<f64>().ok())
                    .map_or(Value::Null, |n| json!(n))
            };
            break;
        }
    }
    json!({"v": 1, "agents": agents.into_values().collect::<Vec<_>>()})
}
