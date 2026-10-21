//! Request-path compression.
//!
//! The proxy sits between Claude Code and the model, so it sees every tool
//! result the agent is about to re-read: a 20 KB file read, a full `cargo
//! build` log, a whole test suite's output. Most of that is not the answer.
//!
//! This module shrinks oversized tool results on the way through and leaves a
//! retrieval handle behind, so the model can pull the original back verbatim
//! when it actually needs the detail. Two rules keep it safe:
//!
//!   * nothing under [`MIN_TOKENS`] is touched — small results are already
//!     cheap and are usually exactly what the agent asked for;
//!   * nothing is replaced unless the compressor saves at least
//!     [`MIN_SAVINGS`], because a marker that costs more than it saves is
//!     worse than forwarding the original.
//!
//! Error results are never compressed: an exception message is the answer, not
//! context noise.

use std::path::{Path, PathBuf};

use imc_core::settings;
use imc_proxy_types::types::anthropic::{
    AnthropicInputMessage, AnthropicToolResultContentBlock, AnthropicUserContentBlock,
    StringOrBlocks,
};

use crate::compression::{ccr_store::CcrStore, compress, kind_slug};

/// Tool results smaller than this are forwarded untouched (tokens, ≈4 B each).
pub const MIN_TOKENS: usize = 400;
/// Minimum fraction of tokens the compressor must save before we swap.
pub const MIN_SAVINGS: f64 = 0.20;

/// What a pass over the conversation did.
#[derive(Debug, Default, Clone, Copy)]
pub struct CompressionReport {
    /// Tool result blocks inspected.
    pub considered: usize,
    /// Blocks actually replaced.
    pub compressed: usize,
    pub original_tokens: usize,
    pub compressed_tokens: usize,
}

impl CompressionReport {
    pub fn saved_tokens(&self) -> usize {
        self.original_tokens.saturating_sub(self.compressed_tokens)
    }

    pub fn is_empty(&self) -> bool {
        self.compressed == 0
    }
}

/// Where the CCR store lives. Honours `IMC_CCR_DB` so tests and alternate
/// installs can point it somewhere else.
pub fn ccr_db_path() -> PathBuf {
    if let Some(p) = std::env::var_os("IMC_CCR_DB") {
        return PathBuf::from(p);
    }
    imc_core::settings::path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("proxy_ccr.db")
}

/// Walk every tool result in the conversation and compress the big ones.
pub fn compress_tool_results(
    messages: &mut [AnthropicInputMessage],
    db: &Path,
) -> CompressionReport {
    let mut report = CompressionReport::default();
    let Ok(mut store) = CcrStore::open(db) else {
        // No store means nowhere to put the original: forward untouched rather
        // than compress irreversibly.
        return report;
    };

    // Identify tool_use_ids corresponding to retrieve operations so their output is kept intact.
    let mut retrieve_tool_ids = std::collections::HashSet::new();
    for msg in messages.iter() {
        if let AnthropicInputMessage::Assistant(assistant) = msg {
            if let StringOrBlocks::Blocks(blocks) = &assistant.content {
                for b in blocks {
                    if let imc_proxy_types::types::anthropic::AnthropicAssistantContentBlock::ToolUse(tu) = b {
                        if tu.name.contains("retrieve") {
                            retrieve_tool_ids.insert(tu.id.clone());
                        } else if let Some(cmd) = tu.input.get("command").and_then(|v| v.as_str()) {
                            if cmd.contains("retrieve") {
                                retrieve_tool_ids.insert(tu.id.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    for message in messages.iter_mut() {
        let AnthropicInputMessage::User(user) = message else {
            continue;
        };
        let StringOrBlocks::Blocks(blocks) = &mut user.content else {
            continue;
        };
        for block in blocks.iter_mut() {
            let AnthropicUserContentBlock::ToolResult(result) = block else {
                continue;
            };
            // The error text *is* the answer.
            if result.is_error == Some(true) {
                continue;
            }
            // An explicit retrieve operation means the caller wanted the full original text.
            if retrieve_tool_ids.contains(&result.tool_use_id) {
                continue;
            }
            match &mut result.content {
                StringOrBlocks::Text(text) => {
                    maybe_shrink(text, &mut store, &mut report);
                }
                StringOrBlocks::Blocks(inner) => {
                    for b in inner.iter_mut() {
                        if let AnthropicToolResultContentBlock::Text(t) = b {
                            maybe_shrink(&mut t.text, &mut store, &mut report);
                        }
                    }
                }
            }
        }
    }
    report
}

/// Compress one text payload in place if it is big enough and the win is real.
fn maybe_shrink(text: &mut String, store: &mut CcrStore, report: &mut CompressionReport) {
    report.considered += 1;
    if text.len() < MIN_TOKENS * 4 {
        return;
    }
    // Never re-compress content that is already compressed or already carries an imc CCR handle.
    if (text.contains("[imc: ") && text.contains("full output is id "))
        || text.contains("## Shortened (full text on demand)")
    {
        return;
    }

    let result = compress(text);
    if result.original_tokens == 0 {
        return;
    }
    let saved = result.saved_tokens as f64 / result.original_tokens as f64;
    // Read min_savings from config, fall back to the compile-time default.
    let min_savings = settings::load()
        .compression
        .as_ref()
        .and_then(|c| c.min_savings)
        .unwrap_or(MIN_SAVINGS);
    if saved < min_savings {
        return;
    }

    let id = match store.store(
        kind_slug(result.kind),
        text,
        &result.compressed,
        result.original_tokens as i64,
        result.compressed_tokens as i64,
    ) {
        Ok(id) => id,
        Err(_) => return,
    };

    report.compressed += 1;
    report.original_tokens += result.original_tokens;
    report.compressed_tokens += result.compressed_tokens;
    *text = format!(
        "{}\n\n[imc: {} -> {} tokens; full output is id {}. Retrieve with `imc retrieve {}` or POST /v1/retrieve {{\"id\":\"{}\"}}]",
        result.compressed, result.original_tokens, result.compressed_tokens, id, id, id
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use imc_proxy_types::types::anthropic::{
        AnthropicTextBlock, AnthropicToolResultBlock,
    };
    use serde_json::json;

    fn tmp_db(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("imc-ctx-{tag}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn user_with_tool_result(block: AnthropicToolResultBlock) -> AnthropicInputMessage {
        AnthropicInputMessage::User(imc_proxy_types::types::anthropic::AnthropicUserMessage {
            role: "user".to_string(),
            content: StringOrBlocks::Blocks(vec![AnthropicUserContentBlock::ToolResult(block)]),
        })
    }

    /// Pull the tool-result text out of a one-message conversation.
    fn only_tool_result_text(msgs: &[AnthropicInputMessage]) -> String {
        let AnthropicInputMessage::User(u) = &msgs[0] else {
            panic!("expected a user message")
        };
        let StringOrBlocks::Blocks(b) = &u.content else {
            panic!("expected blocks")
        };
        let AnthropicUserContentBlock::ToolResult(tr) = &b[0] else {
            panic!("expected a tool result")
        };
        match &tr.content {
            StringOrBlocks::Text(s) => s.clone(),
            StringOrBlocks::Blocks(_) => panic!("expected bare text content"),
        }
    }

    fn tool_result(content: StringOrBlocks<AnthropicToolResultContentBlock>) -> AnthropicToolResultBlock {
        AnthropicToolResultBlock {
            r#type: "tool_result".to_string(),
            tool_use_id: "toolu_1".to_string(),
            content,
            is_error: None,
            cache_control: None,
        }
    }

    #[test]
    fn small_tool_result_is_untouched() {
        let db = tmp_db("small");
        let original = "ok".to_string();
        let mut msgs = vec![user_with_tool_result(tool_result(StringOrBlocks::Text(
            original.clone(),
        )))];
        let report = compress_tool_results(&mut msgs, &db);
        assert_eq!(report.compressed, 0);
        assert_eq!(report.considered, 1);
        assert_eq!(only_tool_result_text(&msgs), original);
    }

    #[test]
    fn error_result_is_never_compressed() {
        let db = tmp_db("error");
        let big = "ERROR: could not connect\n".repeat(500);
        let mut tr = tool_result(StringOrBlocks::Text(big.clone()));
        tr.is_error = Some(true);
        let mut msgs = vec![user_with_tool_result(tr)];
        let report = compress_tool_results(&mut msgs, &db);
        assert_eq!(report.compressed, 0, "error output must survive verbatim");
    }

    #[test]
    fn big_repetitive_result_is_compressed_and_retrievable() {
        let db = tmp_db("big");
        // A tool result of the shape a build log or repeated fixture takes.
        let big = "2026-10-19 10:00:00 INFO step complete\n".repeat(300);
        let mut msgs = vec![user_with_tool_result(tool_result(StringOrBlocks::Text(
            big.clone(),
        )))];
        let report = compress_tool_results(&mut msgs, &db);
        assert_eq!(report.compressed, 1, "expected the big result to compress");
        assert!(report.saved_tokens() > 0);

        let new_text = only_tool_result_text(&msgs);
        assert!(new_text.len() < big.len(), "compressed text should be smaller");
        assert!(new_text.contains("/v1/retrieve"), "must leave a retrieval handle");

        // The handle must actually resolve to the original bytes.
        let id_start = new_text.find(r#"{"id":""#).expect("handle must carry an id") + 7;
        let id = &new_text[id_start..id_start + new_text[id_start..].find('"').unwrap()];
        let store = CcrStore::open(&db).unwrap();
        let recovered = store.get(id).unwrap().expect("id must resolve");
        assert_eq!(recovered, big, "CCR must hold the exact original");
    }

    #[test]
    fn block_list_content_is_compressed_too() {
        let db = tmp_db("blocks");
        let big = "fn main() {\n    let x = 1;\n}\n".repeat(200);
        let inner = vec![AnthropicToolResultContentBlock::Text(AnthropicTextBlock {
            r#type: "text".to_string(),
            text: big.clone(),
            cache_control: None,
        })];
        let mut msgs = vec![user_with_tool_result(tool_result(StringOrBlocks::Blocks(
            inner,
        )))];
        let report = compress_tool_results(&mut msgs, &db);
        assert_eq!(report.compressed, 1);
    }

    #[test]
    fn assistant_messages_are_left_alone() {
        let db = tmp_db("assistant");
        let mut msgs = vec![AnthropicInputMessage::Assistant(Default::default())];
        let report = compress_tool_results(&mut msgs, &db);
        assert_eq!(report.considered, 0);
        assert_eq!(report.compressed, 0);
    }

    #[test]
    fn json_object_payload_roundtrips_through_the_report() {
        // Guard the arithmetic the log line depends on.
        let r = CompressionReport {
            considered: 1,
            compressed: 1,
            original_tokens: 1000,
            compressed_tokens: 250,
        };
        assert_eq!(r.saved_tokens(), 750);
        let _ = json!({"saved": r.saved_tokens()});
    }
}
