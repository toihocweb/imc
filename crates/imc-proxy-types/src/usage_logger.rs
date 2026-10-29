//! Port of `main/proxy/lib/usage_logger.py`.
//!
//! Appends one JSON record per request to `usage.json` in the process CWD,
//! behind a lock so concurrent requests can't interleave. Python writes with
//! plain `write_text`; the Rust port uses the shared atomic writer for the same
//! "never leave a truncated file behind" guarantee the rest of the tree uses.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::atomic_file::write_file_atomically;
use crate::logger::create_handler_logger;

/// Where `usage.json` is stored. Stored in IMC root directory (`~/.indexmycode-rs/usage.json`),
/// or overridden via `IMC_USAGE_FILE`.
pub fn usage_log_file() -> PathBuf {
    if let Ok(p) = std::env::var("IMC_USAGE_FILE") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    imc_core::config::Config::default()
        .data_dir
        .join("usage.json")
}

pub const MAX_USAGE_RECORDS: usize = 1000;

static LOCK: Mutex<()> = Mutex::const_new(());

pub fn read_records(path: &Path) -> Vec<Value> {
    if let Ok(content) = std::fs::read_to_string(path) {
        if !content.trim().is_empty() {
            if let Ok(records) = serde_json::from_str::<Vec<Value>>(&content) {
                return records;
            }
        }
    }
    // Fallback: check legacy cwd/usage.json if path does not exist
    if let Ok(cwd) = std::env::current_dir() {
        let legacy = cwd.join("usage.json");
        if legacy != path && legacy.exists() {
            if let Ok(content) = std::fs::read_to_string(&legacy) {
                if !content.trim().is_empty() {
                    if let Ok(records) = serde_json::from_str::<Vec<Value>>(&content) {
                        return records;
                    }
                }
            }
        }
    }
    Vec::new()
}

/// Port of `usage_logger.log_usage`.
pub async fn log_usage(
    model: &str,
    endpoint: &str,
    usage_data: &Value,
    copilot_usage: Option<&Value>,
) -> anyhow::Result<()> {
    let logger = create_handler_logger("usage_logger");

    let cache_read = usage_data
        .get("cache_read_input_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let cache_write = usage_data
        .get("cache_creation_input_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let input_tok = usage_data
        .get("input_tokens")
        .or_else(|| usage_data.get("prompt_tokens"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let output_tok = usage_data
        .get("output_tokens")
        .or_else(|| usage_data.get("completion_tokens"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    // The hit rate is the instrument for prompt-shape regressions: when the
    // injected context changes on every turn, the cached prefix stops
    // matching and this number collapses.
    let cache_info = if cache_read != 0 || cache_write != 0 {
        let total = cache_read + cache_write + input_tok;
        let hit = if total > 0 {
            cache_read as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        format!(
            " | cache_read={cache_read} cache_write={cache_write} (hit {hit:.1}%)"
        )
    } else {
        String::new()
    };
    logger.info(&format!(
        "[Usage] {model} in={input_tok} out={output_tok}{cache_info}"
    ));
    logger.info(&format!(
        "[Usage Raw] usage_data={usage_data} copilot_usage={}",
        copilot_usage
            .map(|c| c.to_string())
            .unwrap_or_else(|| "None".to_string())
    ));

    let _guard = LOCK.lock().await;

    let record = json!({
        "timestamp": crate::utils::iso_format(now_secs()),
        "model": model,
        "endpoint": endpoint,
        "usage": usage_data,
        "copilot_usage": copilot_usage,
    });
    let mut records = read_records(&usage_log_file());
    records.push(record);
    if records.len() > MAX_USAGE_RECORDS {
        let drain_count = records.len() - MAX_USAGE_RECORDS;
        records.drain(0..drain_count);
    }
    let payload = serde_json::to_string_pretty(&records)?;
    let path = usage_log_file();
    write_file_atomically(&path.to_string_lossy(), &payload)?;
    Ok(())
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}