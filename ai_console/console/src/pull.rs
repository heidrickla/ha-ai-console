//! The pullers: the router's calls, cloud state and host health, and the tracker's Home Assistant entities.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::app::App;
use crate::harness::harness_from;

const HA_EVERY: Duration = Duration::from_secs(300);
const HA_DEFAULT: &str = "http://supervisor/core/api";

pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .build()
        .into()
}

fn fetch(agent: &ureq::Agent, url: &str) -> Result<String, String> {
    let mut resp = agent.get(url).call().map_err(|e| e.to_string())?;
    resp.body_mut()
        .with_config()
        .limit(8 * 1024 * 1024)
        .read_to_string()
        .map_err(|e| e.to_string())
}

/// One pull of `/routes`, `/cloud` and `/health`; each failure stays with its feed as an error state.
pub fn pull_router_side(app: &App, agent: &ureq::Agent) {
    let base = app.router_url();
    let routes = format!("{base}/routes?since={}", app.router_since());
    for (name, url) in [
        ("router", routes),
        ("cloud", format!("{base}/cloud")),
        ("health", format!("{base}/health")),
    ] {
        // ingest_pulled records a validation failure itself; a failed fetch is recorded here.
        match fetch(agent, &url) {
            Ok(body) => {
                let _ = app.ingest_pulled(name, &body);
            }
            Err(e) => app.pull_failed(name, &e),
        }
    }
}

/// One read of the tracker's entities: the ids by integration, then their states.
pub fn pull_harness(app: &App, agent: &ureq::Agent, base: &str, token: Option<&str>) {
    let Some(token) = token else {
        app.pull_failed("harness", "no SUPERVISOR_TOKEN: homeassistant_api is off");
        return;
    };
    let auth = format!("Bearer {token}");
    let result = (|| -> Result<Value, String> {
        let template = json!({"template": "{{ integration_entities('agent_harness_performance_tracker') | tojson }}"});
        let mut r = agent
            .post(format!("{base}/template"))
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .send(template.to_string())
            .map_err(|e| e.to_string())?;
        let ids: Vec<String> =
            serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| e.to_string())?)
                .map_err(|e| format!("template answer is not a list: {e}"))?;
        let mut r = agent
            .get(format!("{base}/states"))
            .header("Authorization", &auth)
            .call()
            .map_err(|e| e.to_string())?;
        let body = r
            .body_mut()
            .with_config()
            .limit(32 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| e.to_string())?;
        let states: Vec<Value> =
            serde_json::from_str(&body).map_err(|e| format!("states is not a list: {e}"))?;
        Ok(harness_from(&ids, &states))
    })();
    match result {
        Ok(doc) => app.ingest_harness(doc),
        Err(e) => app.pull_failed("harness", &e),
    }
}

/// Where s6-overlay keeps the container's environment; a CMD started under s6 does not inherit it.
pub const S6_ENV: &str = "/run/s6/container_environment";

/// The Supervisor token: the process environment first, else s6's copy of the container environment.
pub fn supervisor_token(s6_env: &str) -> Option<String> {
    let clean = |t: String| Some(t.trim().to_owned()).filter(|t| !t.is_empty());
    std::env::var("SUPERVISOR_TOKEN")
        .ok()
        .and_then(clean)
        .or_else(|| {
            std::fs::read_to_string(std::path::Path::new(s6_env).join("SUPERVISOR_TOKEN"))
                .ok()
                .and_then(clean)
        })
}

/// The pull schedule, on the monotonic clock.
pub fn run(app: Arc<App>) {
    let agent = agent();
    let (mut last_router, mut last_ha): (Option<Instant>, Option<Instant>) = (None, None);
    let base = std::env::var("AI_CONSOLE_HA_URL").unwrap_or_else(|_| HA_DEFAULT.into());
    loop {
        let due =
            |last: Option<Instant>, every: Duration| last.is_none_or(|t| t.elapsed() >= every);
        if !app.router_url().is_empty()
            && due(last_router, Duration::from_secs(app.poll_interval_s()))
        {
            pull_router_side(&app, &agent);
            last_router = Some(Instant::now());
        }
        if due(last_ha, HA_EVERY) {
            pull_harness(&app, &agent, &base, supervisor_token(S6_ENV).as_deref());
            last_ha = Some(Instant::now());
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}
