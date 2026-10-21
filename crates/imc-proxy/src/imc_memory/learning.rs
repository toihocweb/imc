//! Learning memories from the traffic the proxy sees.
//!
//! The proxy is on the hot path: every token of every request goes through
//! it. So the learning is built to cost the request nothing:
//!
//! - the gate is a handful of substring checks — microseconds, no I/O;
//! - the model call runs on a blocking thread, never on the async runtime;
//! - the request returns before any of it starts;
//! - a cooldown keeps a long session from firing a call per turn;
//! - a cap keeps the number of learning threads bounded;
//! - the call has a timeout, so a slow endpoint cannot pile threads up.
//!
//! What is worth learning is decided the way Python decides it: a correction
//! from the user, or a command that failed. Everything else is routine work,
//! and routine work is not a memory.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use imc_proxy_types::logger::create_handler_logger;
use imc_proxy_types::types::anthropic::{AnthropicInputMessage, StringOrBlocks};

/// How long to wait before learning from the same project again.
const COOLDOWN: Duration = Duration::from_secs(180);
/// How many learning threads may run at once.
const MAX_CONCURRENT: usize = 2;
/// The longest text handed to the model.
const MAX_TEXT: usize = 4_000;

/// The last learning time per project.
fn last_seen() -> &'static Mutex<HashMap<String, Instant>> {
    static LAST: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The number of learning threads running right now.
fn running() -> &'static AtomicUsize {
    static RUNNING: OnceLock<AtomicUsize> = OnceLock::new();
    RUNNING.get_or_init(|| AtomicUsize::new(0))
}

/// The text of the last assistant reply — the answer the user is reacting to.
pub fn last_assistant_text(messages: &[AnthropicInputMessage]) -> String {
    for msg in messages.iter().rev() {
        let imc_proxy_types::types::anthropic::AnthropicInputMessage::Assistant(am) = msg else {
            continue;
        };
        match &am.content {
            StringOrBlocks::Text(s) => {
                if !s.trim().is_empty() {
                    return truncate(s);
                }
            }
            StringOrBlocks::Blocks(blocks) => {
                let text: Vec<String> = blocks
                    .iter()
                    .filter_map(|b| match b {
                        imc_proxy_types::types::anthropic::AnthropicAssistantContentBlock::Text(
                            t,
                        ) => Some(t.text.clone()),
                        _ => None,
                    })
                    .collect();
                let joined = text.join("\n");
                if !joined.trim().is_empty() {
                    return truncate(&joined);
                }
            }
        }
    }
    String::new()
}

fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_TEXT {
        return text.to_string();
    }
    text.chars().take(MAX_TEXT).collect()
}

/// Whether this exchange holds something worth remembering.
///
/// Cheap on purpose: substring checks, no allocation, no I/O. The expensive
/// question is asked only after this says yes.
pub fn is_worth_learning(query: &str, answer: &str) -> bool {
    imc_memory::extract::is_correction(query)
        || imc_memory::extract::has_failure(answer)
        || imc_memory::extract::has_failure(query)
}

/// Learn from an exchange, in the background.
///
/// Returns immediately. The caller — the request path — does not wait for the
/// model, and nothing here can fail the request.
pub fn learn_in_background(project_root: PathBuf, query: String, answer: String) {
    let log = create_handler_logger("main");

    // The gate: no correction, no failure, nothing to learn.
    if !is_worth_learning(&query, &answer) {
        return;
    }

    // The cooldown: one learning call per project every few minutes. A user
    // who corrects three things in a row gets one extraction, not three.
    {
        let mut seen = match last_seen().lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let now = Instant::now();
        if let Some(previous) = seen.get(&project_root.to_string_lossy().to_string()) {
            if now.duration_since(*previous) < COOLDOWN {
                return;
            }
        }
        seen.insert(project_root.to_string_lossy().to_string(), now);
    }

    // The cap: learning threads are bounded, so a burst of sessions cannot
    // flood the machine.
    if running().load(Ordering::SeqCst) >= MAX_CONCURRENT {
        log.debug("[IMC Learning] too many learning threads, skipping");
        return;
    }
    running().fetch_add(1, Ordering::SeqCst);

    // The background: a blocking thread, so the async runtime keeps serving
    // requests while the model thinks. The handle is dropped on purpose —
    // nobody waits for it.
    tokio::task::spawn_blocking(move || {
        learn_now(&project_root, &query, &answer);
        running().fetch_sub(1, Ordering::SeqCst);
    });
}

/// The extraction itself. Runs on its own thread.
fn learn_now(project_root: &PathBuf, query: &str, answer: &str) {
    let log = create_handler_logger("main");
    let cfg = imc_core::config::Config::default();
    let db = cfg.project_db(project_root);
    let store = match imc_core::store::open(&db) {
        Ok(s) => s,
        Err(e) => {
            log.warning(&format!("[IMC Learning] could not open the store: {e}"));
            return;
        }
    };
    match imc_memory::extract::learn_from_exchange(&cfg, &store, query, answer, "auto") {
        Ok(0) => log.debug("[IMC Learning] nothing worth keeping"),
        Ok(n) => log.info(&format!(
            "[IMC Learning] learned {n} memory(ies) for {}",
            project_root.display()
        )),
        Err(e) => log.warning(&format!("[IMC Learning] extraction failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_routine_exchange_is_not_worth_learning() {
        assert!(!is_worth_learning(
            "please run the tests",
            "all 42 tests passed"
        ));
    }

    #[test]
    fn a_correction_is_worth_learning() {
        assert!(is_worth_learning("that's wrong, use --raw", "ok, noted"));
    }

    #[test]
    fn a_failure_is_worth_learning() {
        assert!(is_worth_learning("try again", "Error: command not found"));
    }

    #[test]
    fn long_text_is_truncated() {
        let long = "x".repeat(MAX_TEXT + 100);
        assert_eq!(truncate(&long).chars().count(), MAX_TEXT);
        assert_eq!(truncate("short").chars().count(), 5);
    }
}
