//! `ai-console serve` runs in the add-on: the page, the push endpoints and the pullers.

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;

use ai_console::app::{App, Config};
use ai_console::clock::now_unix;
use ai_console::options;
use ai_console::server::{ServerConfig, spawn};

const USAGE: &str = "usage: ai-console serve [--port 8096] [--options /data/options.json] [--data /data]
Local runs only: AI_CONSOLE_EXTRA_PUSH_SOURCES (comma-separated addresses added to push_sources),
AI_CONSOLE_HA_URL (default http://supervisor/core/api). SUPERVISOR_TOKEN is read for Home Assistant.";

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("serve") {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }
    std::process::exit(serve(&args));
}

fn serve(args: &[String]) -> i32 {
    let port = flag(args, "--port").unwrap_or_else(|| "8096".into());
    let data = PathBuf::from(flag(args, "--data").unwrap_or_else(|| "/data".into()));
    let options_path = flag(args, "--options").unwrap_or_else(|| "/data/options.json".into());
    let text = match std::fs::read_to_string(&options_path) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            eprintln!("ai-console: cannot read {options_path}: {e}");
            return 1;
        }
    };
    let opts = match options::parse(
        text.as_deref(),
        &std::env::var("AI_CONSOLE_EXTRA_PUSH_SOURCES").unwrap_or_default(),
    ) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("ai-console: refusing to start: {e}");
            return 1;
        }
    };
    eprintln!(
        "ai-console: push_sources {:?}, router {}, data {}",
        opts.push_sources,
        if opts.router_url.is_empty() {
            "off"
        } else {
            &opts.router_url
        },
        data.display()
    );
    let listener = match TcpListener::bind(format!("0.0.0.0:{port}")) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("ai-console: cannot listen on {port}: {e}");
            return 1;
        }
    };
    let app = Arc::new(App::new(
        Config::new(data, opts.push_sources, opts.router_url),
        Arc::new(now_unix),
    ));
    let puller = app.clone();
    std::thread::spawn(move || ai_console::pull::run(puller));
    eprintln!("ai-console listening on {port}");
    let _ = spawn(app, listener, ServerConfig::default()).join();
    1
}
