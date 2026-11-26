mod common;
use ai_console::harness::harness_from;
use common::fixture_json;
use serde_json::{Value, json};

fn fixtures() -> (Vec<String>, Vec<Value>) {
    (
        serde_json::from_value(fixture_json("ha-entities.json")).unwrap(),
        serde_json::from_value(fixture_json("ha-states.json")).unwrap(),
    )
}

#[test]
fn agents_are_matched_by_the_trackers_entity_ids() {
    let (ids, states) = fixtures();
    let h = harness_from(&ids, &states);
    let agents = h["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 2, "light.kitchen is not the tracker's");
    let claude = agents
        .iter()
        .find(|a| a["agent"] == "claude_code_on_workstation")
        .unwrap();
    assert_eq!(claude["regressed"], json!("off"));
    assert_eq!(claude["success_rate"], json!(88.4));
    assert_eq!(claude["window_success_rate"], json!(90.0));
    let codex = agents
        .iter()
        .find(|a| a["agent"] == "codex_on_workstation")
        .unwrap();
    assert_eq!(codex["regressed"], json!("on"));
    assert_eq!(codex["success_rate"], json!(80.0));
}

#[test]
fn an_unavailable_state_is_null_and_an_entity_outside_the_integration_is_ignored() {
    let (ids, states) = fixtures();
    let h = harness_from(&ids, &states);
    let codex = h["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["agent"] == "codex_on_workstation")
        .unwrap()
        .clone();
    assert!(codex["window_success_rate"].is_null());
    let only_light = harness_from(&["light.kitchen".to_string()], &states);
    assert_eq!(only_light["agents"], json!([]));
    let mut gone = states.clone();
    gone[0]["state"] = json!("unavailable");
    let h = harness_from(&ids, &gone);
    let claude = h["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["agent"] == "claude_code_on_workstation")
        .unwrap()
        .clone();
    assert_eq!(claude["regressed"], json!("unavailable"));
}
