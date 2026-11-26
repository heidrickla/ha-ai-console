//! The add-on's options, checked at start: a value that is wrong refuses the start with its name.

use std::net::IpAddr;

use serde_json::Value;

#[derive(Debug)]
pub struct Options {
    /// Empty accepts no push.
    pub push_sources: Vec<IpAddr>,
    /// Empty means no router is pulled.
    pub router_url: String,
}

fn address(entry: &str, origin: &str) -> Result<IpAddr, String> {
    entry
        .trim()
        .parse()
        .map_err(|_| format!("{origin} entry {entry:?} is not an IP address"))
}

/// `text` is options.json when it exists; `extra` is the local-run list from the environment.
pub fn parse(text: Option<&str>, extra: &str) -> Result<Options, String> {
    let opts: Value = match text {
        None => Value::Object(Default::default()),
        Some(t) => serde_json::from_str(t).map_err(|e| format!("options.json is not JSON: {e}"))?,
    };
    if !opts.is_object() {
        return Err("options.json must be a JSON object".into());
    }
    let mut push_sources = match opts.get("push_sources") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| {
                v.as_str()
                    .ok_or_else(|| format!("push_sources entry {v} is not a string"))
                    .and_then(|s| address(s, "push_sources"))
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("push_sources must be a list of IP addresses".into()),
    };
    for entry in extra.split(',').filter(|e| !e.trim().is_empty()) {
        push_sources.push(address(entry, "AI_CONSOLE_EXTRA_PUSH_SOURCES")?);
    }
    let router_url = match opts.get("router_url") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(u)) if u.trim().is_empty() => String::new(),
        Some(Value::String(u)) if u.starts_with("http://") || u.starts_with("https://") => {
            u.trim_end_matches('/').to_string()
        }
        Some(_) => return Err("router_url must be an http:// or https:// URL".into()),
    };
    Ok(Options {
        push_sources,
        router_url,
    })
}
