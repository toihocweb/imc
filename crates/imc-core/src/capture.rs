//! Prompt capture — store the full LLM payload the proxy receives.
//!
//! Port of Python's `PromptCaptureManager` (`main/core/prompts.py`). When
//! enabled, every request the proxy handles is written verbatim to
//! `<data dir>/captures/system<N>.json`, so you can read back exactly what the
//! agent sent upstream (system prompt, tools, memory blocks, tool results).
//!
//! Off by default; the toggle lives in `<data dir>/capture_config.json`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::config::Config;

/// Directory holding the captured payloads.
pub fn captures_dir() -> PathBuf {
    Config::default().data_dir.join("captures")
}

/// File holding the on/off state.
pub fn config_path() -> PathBuf {
    Config::default().data_dir.join("capture_config.json")
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
struct CaptureConfig {
    #[serde(default)]
    auto_capture: bool,
}

/// Is prompt capture on?
pub fn is_enabled() -> bool {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str::<CaptureConfig>(&s)
            .map(|c| c.auto_capture)
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// Turn prompt capture on or off.
pub fn set_enabled(enabled: bool) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(&CaptureConfig {
        auto_capture: enabled,
    })?;
    std::fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// One captured payload.
#[derive(Serialize, Clone, Debug)]
pub struct CaptureInfo {
    /// File stem, e.g. `system12`.
    pub id: String,
    pub path: String,
    pub size: u64,
    /// Unix seconds of the last write.
    pub modified: u64,
    /// Model the request asked for, when present.
    pub model: String,
    /// First user-message text, truncated — enough to recognise the capture.
    pub preview: String,
}

fn next_id(dir: &Path) -> String {
    let mut max = 0u32;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(rest) = name.strip_prefix("system") {
                if let Some(num) = rest.strip_suffix(".json") {
                    if let Ok(n) = num.parse::<u32>() {
                        max = max.max(n);
                    }
                }
            }
        }
    }
    format!("system{}", max + 1)
}

/// Write the payload to the capture directory. Returns the file path.
/// Does nothing (and returns `None`) when capture is off.
pub fn record_payload(payload: &Value) -> Option<PathBuf> {
    if !is_enabled() {
        return None;
    }
    let dir = captures_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let target = dir.join(format!("{}.json", next_id(&dir)));
    let body = serde_json::to_string_pretty(payload).ok()?;
    std::fs::write(&target, body).ok()?;
    Some(target)
}

/// Extract a short preview from a captured payload.
fn preview_of(payload: &Value) -> String {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(system) = payload.get("system") {
        if let Some(s) = system.as_str() {
            candidates.push(s.to_string());
        } else if let Some(arr) = system.as_array() {
            for block in arr {
                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                    candidates.push(t.to_string());
                }
            }
        }
    }
    if let Some(messages) = payload.get("messages").and_then(|m| m.as_array()) {
        for msg in messages.iter().rev() {
            if msg.get("role").and_then(|r| r.as_str()) != Some("user") {
                continue;
            }
            let text = match msg.get("content") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join(" "),
                _ => String::new(),
            };
            if !text.trim().is_empty() {
                candidates.push(text);
                break;
            }
        }
    }
    let raw = candidates.join(" ").replace('\n', " ").trim().to_string();
    if raw.chars().count() > 160 {
        let cut: String = raw.chars().take(157).collect();
        format!("{cut}…")
    } else {
        raw
    }
}

/// All captured payloads, newest first.
pub fn list_captures() -> Vec<CaptureInfo> {
    let dir = captures_dir();
    let mut out: Vec<CaptureInfo> = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let meta = entry.metadata().ok();
        let modified = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let payload = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok());
        out.push(CaptureInfo {
            id: stem,
            path: path.to_string_lossy().to_string(),
            size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
            modified,
            model: payload
                .as_ref()
                .and_then(|p| p.get("model"))
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string(),
            preview: payload.as_ref().map(preview_of).unwrap_or_default(),
        });
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then(b.id.cmp(&a.id)));
    out
}

/// Read one captured payload.
pub fn get_capture(id: &str) -> Option<Value> {
    let path = captures_dir().join(format!("{id}.json"));
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Delete one captured payload.
pub fn delete_capture(id: &str) -> bool {
    let path = captures_dir().join(format!("{id}.json"));
    std::fs::remove_file(path).is_ok()
}

/// Delete every captured payload. Returns how many were removed.
pub fn clear_captures() -> usize {
    let dir = captures_dir();
    let mut n = 0;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json")
                && std::fs::remove_file(&path).is_ok()
            {
                n += 1;
            }
        }
    }
    n
}

/// Small helper for callers that only want the summary.
pub fn status() -> Value {
    json!({
        "enabled": is_enabled(),
        "captures": list_captures().len(),
        "dir": captures_dir().to_string_lossy(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_by_default_and_toggles() {
        // The toggle writes to the data dir; use a temp HOME-free path by
        // pointing the config at a temp file through the data dir env var.
        let dir = std::env::temp_dir().join(format!("imc-capture-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("IMC_DATA_DIR", &dir);

        assert!(!is_enabled(), "capture must be off by default");
        set_enabled(true).unwrap();
        assert!(is_enabled());
        set_enabled(false).unwrap();
        assert!(!is_enabled());

        std::env::remove_var("IMC_DATA_DIR");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn preview_picks_the_user_text() {
        let payload = json!({
            "model": "test-model",
            "messages": [
                {"role": "user", "content": "hello world"}
            ]
        });
        let p = preview_of(&payload);
        assert!(p.contains("hello world"), "{p}");
    }
}
