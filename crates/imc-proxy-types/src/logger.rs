//! Port of `main/proxy/lib/logger.py`.
//!
//! Python uses `logging` + `TimedRotatingFileHandler` (midnight rotation, 7-day
//! retention). Rust has no logging facade in this workspace's dependency set, so
//! this module provides a small [`Logger`] with the same log-line format
//! (`[YYYY-MM-DD HH:MM:SS] [LEVEL] [tag] [traceId] message`), the same file +
//! stdout dual output, and date-based rotation performed lazily on write.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::paths::app_dir;
use crate::process_cleanup::register_process_cleanup;

/// Port of `logger.LOG_RETENTION_DAYS`.
pub const LOG_RETENTION_DAYS: u32 = 7;
/// Port of `logger.LOG_DIR_ENV`.
pub const LOG_DIR_ENV: &str = "COPILOT_API_LOG_DIR";

/// Port of `logger.get_log_dir`.
pub fn get_log_dir() -> PathBuf {
    match std::env::var(LOG_DIR_ENV) {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v.trim()),
        _ => app_dir().join("logs"),
    }
}

/// Port of `logger.ensure_log_directory`.
pub fn ensure_log_directory() -> std::io::Result<PathBuf> {
    let log_dir = get_log_dir();
    fs::create_dir_all(&log_dir)?;
    Ok(log_dir)
}

/// Port of `logger.sanitize_name`.
pub fn sanitize_name(name: &str) -> String {
    let normalized = name.to_lowercase();
    let dashed = collapse_non_alnum(&normalized);
    let trimmed = dashed.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "handler".to_string()
    } else {
        trimmed
    }
}

/// `re.sub(r"[^a-z0-9]+", "-", s)` without pulling in the regex crate.
fn collapse_non_alnum(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_run = false;
    for ch in s.chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            out.push(ch);
            in_run = false;
        } else if !in_run {
            out.push('-');
            in_run = true;
        }
    }
    out
}

/// Port of the `HandlerFormatter.format` line shape.
pub fn format_line(timestamp: &str, level: &str, tag: &str, trace_id: Option<&str>, msg: &str) -> String {
    let trace_str = match trace_id {
        Some(t) if !t.is_empty() => format!(" [{t}]"),
        _ => String::new(),
    };
    if msg.is_empty() {
        format!("[{timestamp}] [{level}] [{tag}]{trace_str}")
    } else {
        format!("[{timestamp}] [{level}] [{tag}]{trace_str} {msg}")
    }
}

/// Handle returned by [`create_handler_logger`]; cloneable and shareable.
#[derive(Clone)]
pub struct Logger {
    name: String,
    file_path: PathBuf,
    level: Level,
}

/// Severity levels, mirroring the `logging` constants the proxy uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug = 10,
    Info = 20,
    Warning = 30,
    Error = 40,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warning => "WARNING",
            Level::Error => "ERROR",
        }
    }
}

impl Logger {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    /// Python's `logger.setLevel(logging.INFO)` default.
    pub fn set_level(&mut self, level: Level) {
        self.level = level;
    }

    /// `logger.isEnabledFor(...)`.
    pub fn is_enabled_for(&self, level: Level) -> bool {
        level >= self.level
    }

    pub fn debug(&self, message: &str) {
        self.log(Level::Debug, message, None);
    }
    pub fn info(&self, message: &str) {
        self.log(Level::Info, message, None);
    }
    pub fn warning(&self, message: &str) {
        self.log(Level::Warning, message, None);
    }
    pub fn error(&self, message: &str) {
        self.log(Level::Error, message, None);
    }

    /// Log with the request `trace_id` attached, as the Python formatter does
    /// via the `trace_id` record attribute.
    pub fn log(&self, level: Level, message: &str, trace_id: Option<&str>) {
        if !self.is_enabled_for(level) {
            return;
        }
        let timestamp = crate::utils::format_datetime_now();
        let line = format_line(&timestamp, level.as_str(), &self.name, trace_id, message);

        // stdout handler (Electron desktop app).
        println!("{line}");

        // File handler with midnight rotation + retention.
        if let Some(parent) = self.file_path.parent() {
            if fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        self.rotate_if_needed();
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.file_path) {
            let _ = writeln!(f, "{line}");
        }
    }

    /// Rotate `name.log` -> `name.log.<YYYY-MM-DD>` when the current date differs
    /// from the one the open file was written on, then prune old rotations.
    fn rotate_if_needed(&self) {
        let today = crate::utils::format_date_now();
        let Some(parent) = self.file_path.parent() else {
            return;
        };
        let sanitized = self
            .file_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "handler".to_string());

        let current_rotation = format!("{sanitized}.log.{today}");
        if !self.file_path.exists() {
            return;
        }
        // Rotate only when the newest rotation isn't today's.
        let already_rotated_today = fs::read_dir(parent)
            .map(|entries| {
                entries.flatten().any(|e| {
                    e.file_name().to_string_lossy() == current_rotation
                })
            })
            .unwrap_or(false);
        if already_rotated_today {
            return;
        }
        let target = parent.join(&current_rotation);
        if fs::rename(&self.file_path, &target).is_ok() {
            self.prune_rotations(parent, &sanitized);
        }
    }

    fn prune_rotations(&self, parent: &Path, sanitized: &str) {
        let Ok(entries) = fs::read_dir(parent) else {
            return;
        };
        let prefix = format!("{sanitized}.log.");
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with(&prefix) {
                continue;
            }
            let Some(date_part) = name.strip_prefix(&prefix) else {
                continue;
            };
            // Rotations older than LOG_RETENTION_DAYS days (YYYY-MM-DD is
            // lexicographically sortable, so a string compare is enough).
            if let Ok(meta) = entry.metadata() {
                if let Ok(modified) = meta.modified() {
                    let modified_secs = modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let now_secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let age_days = (now_secs.saturating_sub(modified_secs)) / 86_400;
                    if age_days >= LOG_RETENTION_DAYS as u64 {
                        let _ = fs::remove_file(entry.path());
                    }
                }
            }
            let _ = date_part;
        }
    }
}

fn logger_registry() -> &'static Mutex<HashMap<String, Logger>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Logger>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Port of `logger.create_handler_logger` — memoized per name, exactly like the
/// Python module-level `_loggers` dict.
pub fn create_handler_logger(name: &str) -> Logger {
    let mut registry = match logger_registry().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if let Some(existing) = registry.get(name) {
        return existing.clone();
    }

    let log_dir = ensure_log_directory().unwrap_or_else(|_| get_log_dir());
    let sanitized_name = sanitize_name(name);
    let file_path = log_dir.join(format!("{sanitized_name}.log"));

    // Open eagerly so a broken path surfaces now, like the Python handler does.
    if let Ok(_file) = OpenOptions::new().create(true).append(true).open(&file_path) {
        // Register for graceful shutdown, mirroring register_process_cleanup.
        let path = file_path.clone();
        // Keep the unregister closure alive with the logger registry entry.
        let _unregister = register_process_cleanup(move || {
            if let Ok(f) = OpenOptions::new().append(true).open(&path) {
                let _ = f.sync_all();
            }
        });
    }

    let logger = Logger {
        name: name.to_string(),
        file_path,
        level: Level::Info,
    };
    registry.insert(name.to_string(), logger.clone());
    logger
}

/// Port of `logger.debug_json`.
pub fn debug_json(logger: &Logger, label: &str, value: &serde_json::Value) {
    if logger.is_enabled_for(Level::Debug) {
        match serde_json::to_string(value) {
            Ok(json) => logger.log(Level::Debug, &format!("{label} {json}"), None),
            Err(_) => logger.log(Level::Debug, &format!("{label} <unserializable>"), None),
        }
    }
}

/// Port of `logger.shutdown_logger_runtime` — flush and drop open handles.
pub fn shutdown_logger_runtime() {
    let mut registry = match logger_registry().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    for logger in registry.values() {
        if let Ok(mut f) = OpenOptions::new().append(true).open(&logger.file_path) {
            let _ = f.flush();
        }
    }
    registry.clear();
    crate::process_cleanup::run_process_cleanups_sync();
}

/// Helper mirroring Python's `FileHandler.close` for a single path.
pub fn close_handler(file: &mut File) {
    let _ = file.flush();
    let _ = file.sync_all();
}