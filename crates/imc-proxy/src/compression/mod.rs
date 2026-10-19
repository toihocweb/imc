//! Compression layer for the imc proxy — port of Headroom's context optimization.
//!
//! Content coming *into* the proxy (tool outputs, logs, RAG chunks, file reads,
//! API responses) is compressed before it reaches the LLM. Originals are kept
//! in the CCR store so the model can retrieve full detail on demand
//! (Compress-Cache-Retrieve — nothing is lost).
//!
//! Content type detection is zero-config: JSON arrays → statistical array
//! compression, source code → AST-aware compression, build/test logs → failure
//! + anomaly preservation, git diffs → hunk preservation, plain text → token
//! classification redundancy removal.

use serde_json::Value;
use std::collections::HashMap;

pub mod ccr_store;
pub mod json_compressor;
pub mod log_compressor;
pub mod router;
pub mod text_compressor;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Detected content kind, used to pick the compressor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ContentKind {
    JsonArray,
    JsonObject,
    SourceCode,
    BuildLog,
    GitDiff,
    PlainText,
    Unknown,
}

/// Result of compressing a piece of content.
#[derive(Debug, Clone)]
#[derive(serde::Serialize)]
pub struct CompressionResult {
    pub original: String,
    pub compressed: String,
    pub kind: ContentKind,
    pub original_tokens: usize,
    pub compressed_tokens: usize,
    pub saved_tokens: usize,
    pub compression_ratio: f64,
}

/// Compress `content`, detected automatically. Returns the compressed text plus
/// metadata. On any error the original is returned unchanged with ratio 1.0.
pub fn compress(content: &str) -> CompressionResult {
    let kind = detect_kind(content);
    let original_tokens = estimate_tokens(content);
    let compressed = match kind {
        ContentKind::JsonArray => json_compressor::compress(content),
        ContentKind::SourceCode => code_compressor_placeholder(content),
        ContentKind::BuildLog => log_compressor::compress(content),
        ContentKind::GitDiff => git_diff_compressor_placeholder(content),
        ContentKind::PlainText | ContentKind::JsonObject | ContentKind::Unknown => {
            text_compressor::compress(content)
        }
    };
    let compressed_tokens = estimate_tokens(&compressed);
    let saved_tokens = original_tokens.saturating_sub(compressed_tokens);
    let ratio = if original_tokens > 0 {
        compressed_tokens as f64 / original_tokens as f64
    } else {
        1.0
    };
    CompressionResult {
        original: content.to_string(),
        compressed,
        kind,
        original_tokens,
        compressed_tokens,
        saved_tokens,
        compression_ratio: ratio,
    }
}

/// Detect content kind from the raw text. Zero-config — no caller annotation
/// required.
pub fn detect_kind(content: &str) -> ContentKind {
    let trimmed = content.trim_start();
    if trimmed.starts_with('[') {
        return ContentKind::JsonArray;
    }
    if trimmed.starts_with('{') {
        return ContentKind::JsonObject;
    }
    // Build logs usually contain timestamps / log levels / severity.
    if content.lines().any(|l| {
        l.contains("ERROR")
            || l.contains("WARN")
            || l.contains("INFO")
            || l.contains("DEBUG")
            || l.contains("FAIL")
            || l.contains("panic")
            || l.contains("error:")
    }) {
        return ContentKind::BuildLog;
    }
    // Git diffs start with `diff --git` or contain `+++`/`---` hunks.
    if content.contains("diff --git")
        || content.lines().any(|l| l.starts_with("+++ ") || l.starts_with("--- "))
    {
        return ContentKind::GitDiff;
    }
    // Source code heuristics: braces, semicolons, `fn`/`def`/`class`/`import`.
    if content.contains('{')
        && content.contains(';')
        || content.contains("fn ")
        || content.contains("def ")
        || content.contains("class ")
        || content.contains("import ")
        || content.contains("const ")
    {
        return ContentKind::SourceCode;
    }
    ContentKind::PlainText
}

/// Very rough token estimate (≈ 4 chars per token). Good enough for ratio math.
pub fn estimate_tokens(text: &str) -> usize {
    text.len().max(1) / 4
}

/// Placeholder: AST-aware code compression is wired to tree-sitter in a later
/// pass. For now it falls back to text compression.
fn code_compressor_placeholder(content: &str) -> String {
    text_compressor::compress(content)
}

/// Placeholder: git diff hunk preservation.
fn git_diff_compressor_placeholder(content: &str) -> String {
    text_compressor::compress(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_json_array() {
        assert_eq!(
            detect_kind(r#"[{"tool":"read","file":"a.rs"}]"#),
            ContentKind::JsonArray
        );
    }

    #[test]
    fn detects_json_object() {
        assert_eq!(detect_kind(r#"{"tool":"read"}"#), ContentKind::JsonObject);
    }

    #[test]
    fn detects_build_log() {
        assert_eq!(
            detect_kind("2026-10-19 ERROR: build failed\npanic at src/main.rs:10"),
            ContentKind::BuildLog
        );
    }

    #[test]
    fn detects_git_diff() {
        assert_eq!(
            detect_kind("diff --git a/foo.rs b/foo.rs\n--- a/foo.rs\n+++ b/foo.rs"),
            ContentKind::GitDiff
        );
    }

    #[test]
    fn compression_returns_metadata() {
        let r = compress(r#"[{"a":1},{"a":1},{"a":1}]"#);
        assert!(r.original_tokens > 0);
        assert!(r.compressed_tokens > 0);
        assert_eq!(r.saved_tokens, r.original_tokens - r.compressed_tokens);
        assert!(r.compression_ratio > 0.0);
    }
}
