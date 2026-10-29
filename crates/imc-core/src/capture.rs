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

/// Directory holding the captured payloads. When a `project_root` is given,
/// payloads are saved into `<project_root>/captures/`. Otherwise, falls back
/// to `<data dir>/captures/`.
pub fn captures_dir(project_root: Option<&Path>) -> PathBuf {
    if let Some(root) = project_root {
        root.join("captures")
    } else {
        Config::default().data_dir.join("captures")
    }
}

/// File holding the on/off state.
pub fn config_path() -> PathBuf {
    Config::default().data_dir.join("capture_config.json")
}

pub const CAPTURE_TIMEOUT_SECS: u64 = 5 * 60;

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
struct CaptureConfig {
    #[serde(default)]
    auto_capture: bool,
    #[serde(default)]
    expires_at: Option<u64>,
}

fn read_config() -> Option<CaptureConfig> {
    let path = config_path();
    let s = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<CaptureConfig>(&s).ok()
}

/// Returns remaining seconds until auto-off if capture is currently active with an expiration.
pub fn remaining_secs() -> Option<u64> {
    let c = read_config()?;
    if !c.auto_capture {
        return None;
    }
    let exp = c.expires_at?;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if now >= exp {
        None
    } else {
        Some(exp - now)
    }
}

/// Is prompt capture on? Automatically disables when expired.
pub fn is_enabled() -> bool {
    let Some(c) = read_config() else {
        return false;
    };
    if !c.auto_capture {
        return false;
    }
    if let Some(exp) = c.expires_at {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if now >= exp {
            let _ = set_enabled(false);
            return false;
        }
    }
    true
}

/// Turn prompt capture on or off. When turning on, sets a 5-minute auto-off expiration.
pub fn set_enabled(enabled: bool) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let expires_at = if enabled {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Some(now + CAPTURE_TIMEOUT_SECS)
    } else {
        None
    };
    let body = serde_json::to_string_pretty(&CaptureConfig {
        auto_capture: enabled,
        expires_at,
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
    record_payload_for_project(None, payload)
}

/// Write the payload to `<project_root>/captures/system<N>.json`.
pub fn record_payload_for_project(project_root: Option<&Path>, payload: &Value) -> Option<PathBuf> {
    if !is_enabled() {
        return None;
    }
    let dir = captures_dir(project_root);
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
pub fn list_captures(project_root: Option<&Path>) -> Vec<CaptureInfo> {
    let dir = captures_dir(project_root);
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
pub fn get_capture(id: &str, project_root: Option<&Path>) -> Option<Value> {
    let path = captures_dir(project_root).join(format!("{id}.json"));
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Delete one captured payload.
pub fn delete_capture(id: &str, project_root: Option<&Path>) -> bool {
    let path = captures_dir(project_root).join(format!("{id}.json"));
    std::fs::remove_file(path).is_ok()
}

/// Delete every captured payload. Returns how many were removed.
pub fn clear_captures(project_root: Option<&Path>) -> usize {
    let dir = captures_dir(project_root);
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
pub fn status(project_root: Option<&Path>) -> Value {
    let enabled = is_enabled();
    let remaining = if enabled { remaining_secs() } else { None };
    json!({
        "enabled": enabled,
        "captures": list_captures(project_root).len(),
        "dir": captures_dir(project_root).to_string_lossy(),
        "remaining_secs": remaining,
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

    #[test]
    fn record_payload_with_project_root() {
        let temp = std::env::temp_dir().join(format!("imc-cap-proj-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();
        std::env::set_var("IMC_DATA_DIR", &temp);

        set_enabled(true).unwrap();

        let proj_root = temp.join("my_proj");
        std::fs::create_dir_all(&proj_root).unwrap();

        let payload = json!({"messages": [{"role": "user", "content": "test payload"}]});
        let path = record_payload_for_project(Some(&proj_root), &payload).expect("should write");
        assert!(path.starts_with(proj_root.join("captures")));
        assert!(path.exists());

        let list = list_captures(Some(&proj_root));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "system1");

        let cap = get_capture("system1", Some(&proj_root));
        assert!(cap.is_some());

        assert!(delete_capture("system1", Some(&proj_root)));
        assert_eq!(list_captures(Some(&proj_root)).len(), 0);

        set_enabled(false).unwrap();
        std::env::remove_var("IMC_DATA_DIR");
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn auto_disables_after_expiration() {
        let dir = std::env::temp_dir().join(format!("imc-cap-exp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("IMC_DATA_DIR", &dir);

        set_enabled(true).unwrap();
        assert!(is_enabled());
        let rem = remaining_secs().unwrap();
        assert!(rem > 0 && rem <= CAPTURE_TIMEOUT_SECS);

        // Manually write an expired config
        let path = config_path();
        let expired_config = serde_json::to_string(&CaptureConfig {
            auto_capture: true,
            expires_at: Some(1),
        }).unwrap();
        std::fs::write(&path, expired_config).unwrap();

        // Checking is_enabled() detects expiration, sets disabled, and returns false
        assert!(!is_enabled());
        assert_eq!(remaining_secs(), None);

        std::env::remove_var("IMC_DATA_DIR");
        std::fs::remove_dir_all(&dir).ok();
    }
}
