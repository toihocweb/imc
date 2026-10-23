//! Learning memories from a conversation.
//!
//! Port of Python's `main/memory/extractor.py` (`MemoryExtractor`) and the
//! cheap gates around it. The Python's `bg_extract` was never called by the
//! proxy — the memories there came from `imc learn` and from the copilot
//! router. Here the extraction is a function anyone can call, and the proxy
//! calls it.
//!
//! The prompt is deliberately shorter than Python's: Python segmented the
//! conversation into work *scenes* and then extracted per scene, which needs
//! the whole history. The store has a scene column, so one scene per
//! extraction is enough.

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::{add_typed, list_all, MemoryType};
use imc_core::config::Config;
use imc_core::store::Store;

/// The system prompt for the extraction.
///
/// The rule that matters: only what a reader of the code could not find. A
/// fact that is visible in the source is not a memory — it is a search hit.
pub const EXTRACTION_PROMPT: &str = r#"You extract durable work memories from a coding session.

A memory is worth keeping when it is:
- a mistake that was made and how it was actually fixed,
- a correction, preference or rule the user stated,
- a project-specific convention, command, or workflow required by the user (e.g. 'always use cargo test -p ...', 'never use symlinks in rules', 'format numbers with US locale'),
- a non-obvious gotcha: a trap, an edge case, a behaviour that surprised someone.

A memory is NOT worth keeping when:
- it is visible by reading the code (who extends what, what a function takes,
  the file layout, a standard import),
- it is a convention any programmer already knows,
- the project's own guide files already say it (CLAUDE.md, README, the rules),
- it is a routine action that succeeded without a mistake or a correction.
  Then the answer is [].

Write each memory as one sentence of 15 to 80 words, in the language the
messages use, naming the concrete thing (the file, the command, the setting)
rather than the general idea. "the deploy pipeline must run rustfmt before
commit" is a memory; "the team should follow good practices" is not.

Keep the specifics: proper nouns, numbers, dates and file names are what make
a memory findable later. A relative date ("last Tuesday") is resolved against
the date of the exchange and written out.

Say who said it. "The user requires X" is a rule; "X was recommended" is only
a suggestion, and a suggestion is not a memory.

When in doubt, extract: a memory that is wrong costs a little, a memory that
is missing costs a repeat.

Answer with JSON only, no prose:
{"memories":[{"content":"...","type":"work_fact|work_task|work_method|work_artifact|business_rule","scene":"...","importance":0.5}]}
"scene" is a short label for what the work was about. "importance" is 0 to 1."#;

/// One memory the model proposed.
#[derive(Debug, Clone, PartialEq)]
pub struct LearnedMemory {
    pub content: String,
    pub mem_type: MemoryType,
    pub scene: String,
    pub importance: f64,
}

/// Strip the fences a model wraps JSON in, and anything after the closing
/// fence. Port of Python's `_strip_fences`.
pub fn strip_fences(raw: &str) -> String {
    let mut s = raw.trim();
    if let Some(rest) = s.strip_prefix("```json") {
        s = rest;
    } else if let Some(rest) = s.strip_prefix("```") {
        s = rest;
    }
    if let Some(idx) = s.find("```") {
        s = &s[..idx];
    }
    s.trim().to_string()
}

/// Parse the model's answer into memories.
///
/// Tolerant on purpose: a model that returns one object instead of a list, or
/// a memory without a type, still gives something usable.
pub fn parse_extraction(raw: &str) -> Vec<LearnedMemory> {
    let cleaned = strip_fences(raw);
    let parsed: Value = match serde_json::from_str(&cleaned) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let items = match &parsed {
        Value::Array(items) => items.clone(),
        Value::Object(obj) => match obj.get("memories") {
            Some(Value::Array(items)) => items.clone(),
            _ => vec![parsed.clone()],
        },
        _ => Vec::new(),
    };
    let mut out = Vec::new();
    for item in items {
        let content = item
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if content.is_empty() {
            continue;
        }
        let mem_type = MemoryType::from_str(
            item.get("type")
                .and_then(Value::as_str)
                .unwrap_or("work_fact"),
        );
        let scene = item
            .get("scene")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let importance = item
            .get("importance")
            .and_then(Value::as_f64)
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);
        out.push(LearnedMemory {
            content,
            mem_type,
            scene,
            importance,
        });
    }
    out
}

/// Ask the model what is worth remembering.
///
/// Returns an empty list when there is nothing to learn — that is the normal
/// case, and it is a success, not a failure.
pub fn extract(
    cfg: &Config,
    query: &str,
    answer: &str,
    existing: &str,
) -> Result<Vec<LearnedMemory>> {
    let base = cfg.llm_base_url();
    if base.trim().is_empty() {
        return Ok(Vec::new());
    }
    let key = cfg.llm_api_key();
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let user = format!(
        "[Existing memories]\n{existing}\n\n[New exchange]\nUser: {query}\nAssistant: {answer}\n\n\
         What is worth remembering from this exchange?"
    );
    let body = json!({
        "model": cfg.llm_model(),
        "messages": [
            {"role": "system", "content": EXTRACTION_PROMPT},
            {"role": "user", "content": user},
        ],
        "response_format": {"type": "json_object"},
        "temperature": 0.1,
    });
    // A timeout: a hung endpoint must not hold a thread for ever. The
    // learning runs on its own thread, but a thread is a thread.
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .build(),
    );
    let mut req = agent.post(&url).header("Content-Type", "application/json");
    if !key.is_empty() {
        req = req.header("Authorization", format!("Bearer {key}"));
    }
    let mut response = req
        .send_json(body)
        .map_err(|e| anyhow::anyhow!("the extraction call failed: {e}"))?;
    let raw: Value = response
        .body_mut()
        .read_json()
        .unwrap_or(Value::Null);
    let content = raw
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("");
    Ok(parse_extraction(content))
}

/// The existing memories, as the text the prompt shows the model.
pub fn existing_as_text(store: &Store) -> Result<String> {
    // The top 40, not the whole store: the prompt is context the model pays
    // for, and 200 arbitrary memories are mostly noise.
    let items = list_all(store, 40)?;
    Ok(items
        .iter()
        .map(|m| format!("- [ID: {}] ({}): {}", m.id, m.mem_type.as_str(), m.content))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Store what was learned.
///
/// A memory that is already there (same words, or the same fact written
/// differently) reinforces the old one instead of piling up copies.
pub fn store_learned(store: &Store, learned: &[LearnedMemory], source: &str) -> Result<usize> {
    let mut stored = 0;
    for m in learned {
        let id = add_typed(
            store,
            &m.content,
            m.mem_type,
            &m.scene,
            "note",
            &[],
            &[],
            m.importance,
            source,
        )?;
        if id.is_some() {
            stored += 1;
        }
    }
    Ok(stored)
}

/// The whole flow: extract, then store.
///
/// The store is the only state that matters afterwards, so the caller does
/// not have to know the two steps apart.
pub fn learn_from_exchange(
    cfg: &Config,
    store: &Store,
    query: &str,
    answer: &str,
    source: &str,
) -> Result<usize> {
    let existing = existing_as_text(store)?;
    let learned = extract(cfg, query, answer, &existing)?;
    if learned.is_empty() {
        return Ok(0);
    }
    store_learned(store, &learned, source)
}

// ---------------------------------------------------------------------------
// The cheap gates
//
// The extraction costs an LLM call, so it only runs when the exchange holds
// something worth learning. Both gates are the Python's patterns, ported.
// ---------------------------------------------------------------------------

/// A correction or an explicit rule from the user.
pub fn is_correction(text: &str) -> bool {
    let t = text.to_lowercase();
    const PATTERNS: &[&str] = &[
        "that's wrong",
        "that is wrong",
        "this is wrong",
        "you're wrong",
        "you are wrong",
        "that's incorrect",
        "do not use",
        "don't use",
        "do not run",
        "don't run",
        "do not repeat",
        "instead use",
        "use this instead",
        "stop doing",
        "never do",
        "always use",
        "always do",
        "remember to",
        "remember that",
        "keep in mind",
        "note that",
        "must use",
        "must not",
        "should not",
        "the rule is",
        "convention is",
        "sai rồi",
        "không đúng",
        "bạn sai",
        "đừng",
        "không được",
        "thay vào đó",
        "đừng bao giờ",
        "hãy luôn",
        "nhớ là",
        "từ giờ",
        "không dùng",
        "quy tắc là",
        "chú ý",
        "phải luôn",
    ];
    PATTERNS.iter().any(|p| t.contains(p))
}

/// A command that failed.
pub fn has_failure(text: &str) -> bool {
    let t = text.to_lowercase();
    const PATTERNS: &[&str] = &[
        "command not found",
        "no such file or directory",
        "permission denied",
        "traceback (most recent call last)",
        "uncaught exception",
        "fatal error",
    ];
    PATTERNS.iter().any(|p| t.contains(p))
        || t.lines().any(|l| {
            let l = l.trim_start();
            l.starts_with("error:") || l.starts_with("error ")
                || l.starts_with("failed:") || l.starts_with("failure:")
        })
}

/// Whether an exchange is worth an extraction call.
pub fn is_worth_learning(query: &str, answer: &str) -> bool {
    is_correction(query) || has_failure(answer) || has_failure(query)
}

/// Redact the obvious secrets before a memory is stored.
///
/// A memory is injected into every future conversation, so a key that slips
/// into one is a key that leaks into all of them. Port of Python's
/// `redact_sensitive`.
pub fn redact_sensitive(text: &str) -> String {
    let mut out = text.to_string();

    // A secret that appears on its own, without a key next to it. The
    // patterns are the shapes the providers actually hand out.
    for (needle, len) in [
        ("sk-ant-", 40),
        ("sk-", 32),
        ("ghp_", 36),
        ("gho_", 36),
        ("ghu_", 36),
        ("github_pat_", 60),
        ("AKIA", 20),
        ("xoxb-", 24),
        ("xoxp-", 24),
        ("xoxa-", 24),
        ("xoxr-", 24),
        ("xoxs-", 24),
        ("AIza", 39),
        ("eyJ", 120),
    ] {
        let mut from = 0;
        while let Some(idx) = out[from..].find(needle) {
            let start = from + idx;
            let tail = &out[start..];
            let end = tail
                .char_indices()
                .take_while(|(i, ch)| {
                    *i < len && (ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
                })
                .map(|(i, ch)| i + ch.len_utf8())
                .last()
                .unwrap_or(needle.len());
            if end > needle.len() {
                out.replace_range(start..start + end, "[REDACTED]");
            }
            from = start + "[REDACTED]".len();
        }
    }

    // A secret named by a key: `api_key = ...`, `token: ...`.
    for prefix in [
        "api_key", "api-key", "apikey", "access_key", "access-key", "token",
        "password", "passwd", "pwd", "secret", "cookie", "session",
    ] {
        // `prefix = value` or `prefix: value`
        let lower = out.to_lowercase();
        let mut from = 0;
        while let Some(idx) = lower[from..].find(prefix) {
            let start = from + idx;
            let rest = &out[start + prefix.len()..];
            let trimmed = rest.trim_start();
            let gap = rest.len() - trimmed.len();
            if trimmed.starts_with('=') || trimmed.starts_with(':') {
                // The value may be separated from the key by spaces:
                // `api_key = sk-...`.
                let after = &trimmed[1..];
                let value_offset = after.len() - after.trim_start().len();
                let value = &after[value_offset..];
                let value_len = value
                    .find(|c: char| c.is_whitespace() || c == ',' || c == ';')
                    .unwrap_or(value.len());
                let value_start = start + prefix.len() + gap + 1 + value_offset;
                let value_end = value_start + value_len;
                if value_end > value_start {
                    out.replace_range(value_start..value_end, "[REDACTED]");
                }
            }
            from = start + prefix.len();
        }
    }
    out
}

/// Store one lesson the user states, without asking a model.
///
/// Used by `imc learn` when the lesson is already a sentence.
pub fn store_lesson(store: &Store, lesson: &str, scene: &str) -> Result<Option<i64>> {
    let content = redact_sensitive(lesson.trim());
    if content.is_empty() {
        return Ok(None);
    }
    add_typed(
        store,
        &content,
        MemoryType::WorkFact,
        scene,
        "note",
        &[],
        &[],
        0.7,
        "manual",
    )
    .context("store the lesson")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_json_object_becomes_memories() {
        let raw = r#"{"memories":[{"content":"the deploy must run rustfmt before commit","type":"work_method","scene":"deployment","importance":0.8}]}"#;
        let memories = parse_extraction(raw);
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].mem_type, MemoryType::WorkMethod);
        assert_eq!(memories[0].scene, "deployment");
        assert_eq!(memories[0].importance, 0.8);
    }

    #[test]
    fn fences_and_prose_around_the_json_are_tolerated() {
        let raw = "```json\n{\"memories\":[{\"content\":\"a fact\"}]}\n```\nHope this helps!";
        let memories = parse_extraction(raw);
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].content, "a fact");
    }

    #[test]
    fn a_bare_list_is_tolerated() {
        let raw = r#"[{"content":"one"},{"content":"two"}]"#;
        assert_eq!(parse_extraction(raw).len(), 2);
    }

    #[test]
    fn nothing_to_learn_is_an_empty_list_not_an_error() {
        assert!(parse_extraction("{\"memories\":[]}").is_empty());
        assert!(parse_extraction("not json at all").is_empty());
        assert!(parse_extraction("").is_empty());
    }

    #[test]
    fn a_memory_without_a_type_is_a_work_fact() {
        let raw = r#"{"memories":[{"content":"something happened"}]}"#;
        assert_eq!(parse_extraction(raw)[0].mem_type, MemoryType::WorkFact);
    }

    #[test]
    fn corrections_are_detected_in_both_languages() {
        assert!(is_correction("that's wrong, use the other flag"));
        assert!(is_correction("đừng dùng flag đó"));
        assert!(is_correction("do not run cargo publish"));
        assert!(!is_correction("please run the tests"));
    }

    #[test]
    fn failures_are_detected() {
        assert!(has_failure("bash: cargo: command not found"));
        assert!(has_failure("Traceback (most recent call last):"));
        assert!(has_failure("Error: no such column"));
        assert!(!has_failure("all 42 tests passed"));
    }

    #[test]
    fn a_routine_exchange_is_not_worth_learning() {
        assert!(!is_worth_learning("run the tests please", "all 42 tests passed"));
        assert!(is_worth_learning("that's wrong", "ok"));
        assert!(is_worth_learning("try it", "Error: command not found"));
    }

    #[test]
    fn secrets_are_redacted_before_a_memory_is_stored() {
        let text = "the api_key = sk-abcdef123456 and the token: ghp_zzzzzzzz";
        let redacted = redact_sensitive(text);
        assert!(!redacted.contains("sk-abcdef123456"), "{redacted}");
        assert!(!redacted.contains("ghp_zzzzzzzz"), "{redacted}");
        assert!(redacted.contains("[REDACTED]"), "{redacted}");
    }

    /// A secret on its own, with no key next to it, is still a secret. These
    /// three shapes all leaked through the key-based redaction.
    #[test]
    fn bare_tokens_are_redacted() {
        // Assembled at run time: a literal that looks like a key would be
        // flagged by the secret scanner, and rightly so.
        let anthropic = format!("sk-ant-{}", "api03-DDDD");
        let github = format!("ghp_{}", "1234567890abcdefghij");
        let aws = format!("AKIA{}", "IOSFODNN7EXAMPLE");
        let text = format!("the key is {anthropic} and github {github} and aws {aws}");
        let redacted = redact_sensitive(&text);
        assert!(!redacted.contains(&anthropic), "{redacted}");
        assert!(!redacted.contains(&github), "{redacted}");
        assert!(!redacted.contains(&aws), "{redacted}");
        assert_eq!(redacted.matches("[REDACTED]").count(), 3, "{redacted}");
    }

    #[test]
    fn a_jwt_is_redacted() {
        let header = format!("eyJ{}", "hbGciOiJIUzI1NiI");
        let text = format!("auth: {header}.eyJzdWIiOiIxIn0.abc");
        let redacted = redact_sensitive(&text);
        assert!(!redacted.contains(&header), "{redacted}");
    }
}
