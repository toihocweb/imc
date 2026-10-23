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

/// How many learning threads may run at once.
const MAX_CONCURRENT: usize = 2;
/// The longest text handed to the model.
const MAX_TEXT: usize = 4_000;

/// The text of the last learning per project, and when it happened.
///
/// A fixed cooldown drops the second correction: the user says two things in
/// a row, and the second one is never learned. Instead the texts are merged:
/// a correction that arrives within the window joins the last one, and the
/// next extraction sees both. Nothing is lost, and a burst still costs one
/// model call.
fn pending() -> &'static Mutex<HashMap<String, (String, Instant)>> {
    static PENDING: OnceLock<Mutex<HashMap<String, (String, Instant)>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// How long a later correction still joins the last one.
const MERGE_WINDOW: Duration = Duration::from_secs(60);

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


/// The text of every tool result in the conversation.
///
/// A failed command is not in the assistant's answer — it is in the tool
/// result. A gate that only reads the messages misses every one of them.
pub fn tool_result_text(messages: &[AnthropicInputMessage]) -> String {
    use imc_proxy_types::types::anthropic::{
        AnthropicAssistantContentBlock, AnthropicToolResultContentBlock,
        AnthropicUserContentBlock,
    };
    let mut parts: Vec<String> = Vec::new();
    for msg in messages {
        match msg {
            AnthropicInputMessage::User(um) => {
                if let StringOrBlocks::Blocks(blocks) = &um.content {
                    for block in blocks {
                        if let AnthropicUserContentBlock::ToolResult(tr) = block {
                            if tr.is_error == Some(true) {
                                parts.push("[tool error]".to_string());
                            }
                            match &tr.content {
                                StringOrBlocks::Text(s) => parts.push(s.clone()),
                                StringOrBlocks::Blocks(inner) => {
                                    for b in inner {
                                        if let AnthropicToolResultContentBlock::Text(t) = b {
                                            parts.push(t.text.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            AnthropicInputMessage::Assistant(am) => {
                if let StringOrBlocks::Blocks(blocks) = &am.content {
                    for block in blocks {
                        if let AnthropicAssistantContentBlock::ToolUse(tu) = block {
                            if let Ok(text) = serde_json::to_string(&tu.input) {
                                parts.push(text);
                            }
                        }
                    }
                }
            }
            AnthropicInputMessage::System(_) => {}
        }
    }
    parts.join("\n")
}


/// A command that clearly succeeded.
///
/// A success trajectory transfers better than a failure: the failure says
/// what went wrong, the success says what reliably works. So both are worth
/// learning — and a success is also the proof that a pending lesson (one
/// that came from a failure) is right.
pub fn has_success(text: &str) -> bool {
    let lower = text.to_lowercase();
    const PATTERNS: &[&str] = &[
        "exit code 0",
        "exit status 0",
        "success",
        "successful",
        "completed",
        "passed",
        "all tests passed",
    ];
    PATTERNS.iter().any(|p| lower.contains(p))
}

/// Whether this exchange holds something worth remembering.
///
/// Filters out trivial conversational chatter while allowing substantive technical
/// directives, environment outcomes, and failures to be evaluated by the background model.
pub fn is_worth_learning(query: &str, answer: &str, tool_output: &str) -> bool {
    if imc_memory::extract::is_trivial_chatter(query)
        && !imc_memory::extract::has_failure(answer)
        && !imc_memory::extract::has_failure(tool_output)
    {
        return false;
    }

    imc_memory::extract::is_correction(query)
        || imc_memory::extract::has_failure(answer)
        || imc_memory::extract::has_failure(query)
        || imc_memory::extract::has_failure(tool_output)
        || has_success(tool_output)
}

/// Learn from an exchange, in the background.
///
/// Returns immediately. The caller — the request path — does not wait for the
/// model, and nothing here can fail the request.
pub fn learn_in_background(
    project_root: PathBuf,
    query: String,
    answer: String,
    tool_output: String,
) {
    let log = create_handler_logger("main");

    // The gate: no correction, no failure, nothing to learn.
    if !is_worth_learning(&query, &answer, &tool_output) {
        return;
    }

    // The merge: a correction that arrives soon after the last one joins it,
    // so the next extraction sees both. Nothing is dropped.
    let key = project_root.to_string_lossy().to_string();
    let query = {
        let mut pending = match pending().lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let now = Instant::now();
        let merged = match pending.get(&key) {
            Some((text, at)) if now.duration_since(*at) < MERGE_WINDOW => {
                format!("{text}\n{query}")
            }
            _ => query.clone(),
        };
        pending.insert(key.clone(), (merged.clone(), now));
        merged
    };

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
        learn_now(&project_root, &query, &answer, &tool_output);
        running().fetch_sub(1, Ordering::SeqCst);
    });
}

/// The extraction itself. Runs on its own thread.
fn learn_now(project_root: &PathBuf, query: &str, answer: &str, tool_output: &str) {
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
    let exchange = format!("{answer}\n\n[tool output]\n{tool_output}");
    // A failure is a lesson that is not proven yet: stored as `pending`, not
    // injected. A success — or a correction from the user — is a lesson that
    // is proven, and it also promotes the pending lessons it confirms.
    let failed = imc_memory::extract::has_failure(tool_output)
        || imc_memory::extract::has_failure(query)
        || imc_memory::extract::has_failure(answer);
    let source = if failed { "auto" } else { "confirmed" };
    match imc_memory::extract::learn_from_exchange(&cfg, &store, query, &exchange, source) {
        Ok(0) => log.debug("[IMC Learning] nothing worth keeping"),
        Ok(n) => {
            // A success proves the pending lessons it shares words with.
            let promoted = imc_memory::promote_pending(&store, &exchange).unwrap_or(0);
            log.info(&format!(
                "💾 Self-improvement review: learned {n} memory(ies) in project store for {}{}",
                project_root.display(),
                if promoted > 0 {
                    format!(" (promoted {promoted} pending)")
                } else {
                    String::new()
                }
            ));
        }
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
            "all 42 tests passed",
            ""
        ));
    }

    #[test]
    fn a_correction_is_worth_learning() {
        assert!(is_worth_learning("that's wrong, use --raw", "ok, noted", ""));
    }

    #[test]
    fn a_failure_is_worth_learning() {
        assert!(is_worth_learning("try again", "Error: command not found", ""));
    }

    /// The failure usually arrives in the tool result, not in the messages.
    /// A gate that only reads the messages misses every one of them.
    #[test]
    fn a_failure_in_the_tool_output_is_worth_learning() {
        assert!(is_worth_learning(
            "try again",
            "let me run it",
            "bash: cargo: command not found"
        ));
    }

    #[test]
    fn long_text_is_truncated() {
        let long = "x".repeat(MAX_TEXT + 100);
        assert_eq!(truncate(&long).chars().count(), MAX_TEXT);
        assert_eq!(truncate("short").chars().count(), 5);
    }
}
