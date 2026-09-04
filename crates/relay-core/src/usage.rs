//! Bounded, read-only provider usage discovery.
//!
//! Both CLIs already receive rate-limit data while doing normal work. Relay reads the
//! small state they leave behind only when `usage.get` is requested. It never polls a
//! provider, calls a usage endpoint, or spends a provider token to populate chrome.

use relay_bus::types::{Provider, Usage};
use serde_json::{json, Map, Value};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_STATE_FILE: u64 = 2 * 1024 * 1024;
const MAX_CODEX_FILES: usize = 32;
const CODEX_TAIL: u64 = 1024 * 1024;

pub fn read(provider: Provider) -> Option<Usage> {
    match provider {
        Provider::Claude => read_claude(),
        Provider::Codex => read_codex(),
    }
}

fn home(variable: &str, fallback_dir: &str) -> Option<PathBuf> {
    std::env::var_os(variable).filter(|value| !value.is_empty()).map(PathBuf::from).or_else(|| {
        std::env::var_os("HOME").map(PathBuf::from).map(|path| path.join(fallback_dir))
    })
}

fn read_claude() -> Option<Usage> {
    let root = std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))?;
    let path = root.join(".claude").join("relay-usage.json");
    if fs::metadata(&path).ok()?.len() > MAX_STATE_FILE { return None; }
    let raw = fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    let limits = find_named(&value, &["rate_limits", "rateLimits"], 0)?;
    let windows = normalize_group(limits);
    if windows.is_empty() { return None; }
    let taken_at = value.get("timestamp").and_then(Value::as_str)
        .map(str::to_string).unwrap_or_else(crate::time::now);
    Some(Usage { provider: Provider::Claude, windows: Value::Object(windows), taken_at })
}

fn read_codex() -> Option<Usage> {
    let root = home("CODEX_HOME", ".codex")?.join("sessions");
    let mut files = Vec::new();
    collect_jsonl(&root, 0, &mut files);
    files.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));
    for (path, _) in files.into_iter().take(MAX_CODEX_FILES) {
        let raw = read_tail(&path, CODEX_TAIL).ok()?;
        for line in raw.lines().rev() {
            let Ok(value) = serde_json::from_str::<Value>(line) else { continue };
            let limits = value.pointer("/payload/rate_limits")
                .or_else(|| value.pointer("/payload/rateLimits"));
            let Some(limits) = limits.filter(|value| !value.is_null()) else { continue };
            let windows = normalize_codex(limits);
            if windows.is_empty() { continue; }
            let taken_at = value.get("timestamp").and_then(Value::as_str)
                .map(str::to_string).unwrap_or_else(crate::time::now);
            return Some(Usage { provider: Provider::Codex, windows: Value::Object(windows), taken_at });
        }
    }
    None
}

fn collect_jsonl(dir: &Path, depth: u8, out: &mut Vec<(PathBuf, SystemTime)>) {
    if depth > 4 || out.len() > 256 { return; }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() { collect_jsonl(&path, depth + 1, out); }
        else if path.extension().is_some_and(|extension| extension == "jsonl") {
            let modified = entry.metadata().and_then(|metadata| metadata.modified()).unwrap_or(UNIX_EPOCH);
            out.push((path, modified));
        }
    }
}

fn read_tail(path: &Path, max: u64) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(max);
    file.seek(SeekFrom::Start(start))?;
    let mut raw = String::new();
    file.read_to_string(&mut raw)?;
    if start > 0 {
        raw = raw.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or_default();
    }
    Ok(raw)
}

fn find_named<'a>(value: &'a Value, names: &[&str], depth: u8) -> Option<&'a Value> {
    if depth > 8 { return None; }
    match value {
        Value::Object(map) => names.iter().find_map(|name| map.get(*name))
            .or_else(|| map.values().find_map(|child| find_named(child, names, depth + 1))),
        Value::Array(values) => values.iter().find_map(|child| find_named(child, names, depth + 1)),
        _ => None,
    }
}

fn normalize_group(value: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    let Some(group) = value.as_object() else { return out };
    for (name, value) in group {
        if let Some(window) = normalize_window(value) { out.insert(clean_name(name), window); }
        if let Some(items) = value.as_array() {
            for item in items {
                let label = item.pointer("/scope/model/display_name").and_then(Value::as_str)
                    .or_else(|| item.get("name").and_then(Value::as_str));
                if let (Some(label), Some(window)) = (label, normalize_window(item)) {
                    out.entry(clean_name(label)).or_insert(window);
                }
            }
        }
    }
    out
}

fn normalize_codex(value: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    let Some(group) = value.as_object() else { return out };
    for key in ["primary", "secondary", "individual_limit"] {
        let Some(window) = group.get(key).filter(|value| !value.is_null()) else { continue };
        let minutes = window.get("window_minutes").and_then(Value::as_u64);
        let name = match minutes {
            Some(300) => "five_hour".to_string(),
            Some(10_080) => "weekly".to_string(),
            Some(value) => format!("{value}_minute"),
            None => key.to_string(),
        };
        if let Some(window) = normalize_window(window) { out.insert(name, window); }
    }
    out
}

fn normalize_window(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    let used = ["used_pct", "used_percent", "used_percentage", "utilization", "percent"]
        .iter().find_map(|key| object.get(*key).and_then(number))?;
    let resets = ["resets_at", "resetsAt", "reset_at", "resetAt"]
        .iter().find_map(|key| object.get(*key)).and_then(reset_label);
    Some(json!({"used_pct": used.clamp(0.0, 100.0).round(), "resets_in": resets}))
}

fn number(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_str()?.trim_end_matches('%').parse().ok())
}

fn reset_label(value: &Value) -> Option<String> {
    let epoch = value.as_f64().or_else(|| value.as_str()?.parse::<f64>().ok())?;
    let seconds = if epoch > 100_000_000_000.0 { epoch / 1000.0 } else { epoch };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs_f64();
    let remaining = seconds.saturating_sub(now) as u64;
    let days = remaining / 86_400;
    let hours = (remaining % 86_400) / 3600;
    let minutes = (remaining % 3600) / 60;
    Some(if days > 0 { format!("{days}d {hours}h") } else if hours > 0 { format!("{hours}h {minutes}m") } else { format!("{minutes}m") })
}

fn clean_name(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}

trait SaturatingSubF64 { fn saturating_sub(self, rhs: Self) -> Self; }
impl SaturatingSubF64 for f64 { fn saturating_sub(self, rhs: Self) -> Self { (self - rhs).max(0.0) } }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_claude_and_codex_shapes() {
        let claude = normalize_group(&json!({
            "five_hour":{"used_percentage":32,"resets_at":4102444800_u64},
            "seven_day":{"utilization":71}
        }));
        assert_eq!(claude["five_hour"]["used_pct"], 32.0);
        assert_eq!(claude["seven_day"]["used_pct"], 71.0);
        let codex = normalize_codex(&json!({
            "primary":{"used_percent":44,"window_minutes":10080,"resets_at":4102444800_u64},
            "secondary":null
        }));
        assert_eq!(codex["weekly"]["used_pct"], 44.0);
    }
}
