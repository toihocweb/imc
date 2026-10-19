//! Compression layer for the imc proxy — context optimization.
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

pub mod ccr_store;
pub mod code_compressor;
pub mod context;
pub mod git_diff_compressor;
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
    let candidate = match kind {
        ContentKind::JsonArray => json_compressor::compress(content),
        ContentKind::SourceCode => code_compressor::compress(content),
        ContentKind::BuildLog => log_compressor::compress(content),
        ContentKind::GitDiff => git_diff_compressor::compress(content),
        ContentKind::PlainText | ContentKind::JsonObject | ContentKind::Unknown => {
            text_compressor::compress(content)
        }
    };
    // Never inflate: markers and framing can cost more than they save on small
    // inputs, in which case the original is strictly better.
    let candidate_tokens = estimate_tokens(&candidate);
    let (compressed, compressed_tokens) = if candidate_tokens < original_tokens {
        (candidate, candidate_tokens)
    } else {
        (content.to_string(), original_tokens)
    };
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
///
/// Order matters: the strongest, least ambiguous signals are tested first.
/// Diff/log detection used to be a substring scan, which misclassified source
/// files that merely *mention* `error` — and a misclassified file gets the
/// wrong compressor, which destroys it.
pub fn detect_kind(content: &str) -> ContentKind {
    let trimmed = content.trim_start();

    // JSON: unambiguous when it actually parses.
    if trimmed.starts_with('[') {
        if let Ok(Value::Array(_)) = serde_json::from_str::<Value>(content) {
            return ContentKind::JsonArray;
        }
    }
    if trimmed.starts_with('{') && serde_json::from_str::<Value>(content).is_ok() {
        return ContentKind::JsonObject;
    }

    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len().max(1);

    // Git diff: a `diff --git` header is decisive, `git show` output included.
    if content.contains("diff --git ") {
        return ContentKind::GitDiff;
    }
    let hunks = lines.iter().filter(|l| l.starts_with("@@ ")).count();
    let file_headers = lines
        .iter()
        .filter(|l| l.starts_with("+++ ") || l.starts_with("--- "))
        .count();
    if hunks > 0 && file_headers >= 2 {
        return ContentKind::GitDiff;
    }

    // Build log vs source code: count lines that *begin* like a log record
    // against lines that *look* like statements. Prefix-anchored matching is
    // what keeps `// handle error:` inside a source file from looking like a
    // log.
    let log_lines = lines.iter().filter(|l| is_log_line(l)).count();
    let code_lines = lines.iter().filter(|l| is_code_line(l)).count();

    if log_lines * 2 >= total && log_lines > code_lines {
        return ContentKind::BuildLog;
    }
    if code_lines > 0 {
        return ContentKind::SourceCode;
    }
    ContentKind::PlainText
}

/// A line that starts like a build/test log record.
fn is_log_line(line: &str) -> bool {
    let t = line.trim_start();
    // Compiler / toolchain verbs.
    for verb in [
        "Compiling ", "Finished ", "Running ", "Building ", "Downloaded ", "Downloading ",
        "Checking ", "Blocking ", "Fresh ", "warning: ", "error: ", "error[", "note: ",
        "help: ", "--> ", "= note:", "= help:", "test result: ", "FAILED ", "ok ",
        "FAIL ", "PASS ", "panic: ",
    ] {
        if t.starts_with(verb) {
            return true;
        }
    }
    // Bare pipe / caret gutter lines that only appear in diagnostics.
    if t == "|" || t.starts_with("| ") || t.starts_with("^") {
        return true;
    }
    // `LEVEL message` with an uppercase level word.
    for level in ["ERROR", "WARN", "INFO", "DEBUG", "TRACE", "FATAL", "NOTICE"] {
        if t.starts_with(level) || t.starts_with(&format!("[{level}]")) {
            return true;
        }
        // `2026-10-19 10:00:01 ERROR ...` — level right after a timestamp.
        if let Some(idx) = t.find(level) {
            if idx > 0 && idx < 32 {
                let before = t[..idx].trim_end();
                if before.ends_with(|c: char| c.is_ascii_digit() || c == 'T' || c == 'Z') {
                    return true;
                }
            }
        }
    }
    false
}

/// A line that looks like a source statement rather than prose.
fn is_code_line(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    if t.starts_with("//") || t.starts_with('#') || t.starts_with("/*") || t.starts_with('*') {
        return true;
    }
    if t.ends_with(';') || t.ends_with('{') || t.ends_with('}') || t == "}" || t == "{" {
        return true;
    }
    for kw in [
        "fn ", "let ", "pub ", "impl ", "use ", "mod ", "struct ", "enum ", "trait ",
        "match ", "if ", "else ", "for ", "while ", "return ", "def ", "class ",
        "import ", "from ", "const ", "var ", "function ", "=>", "->", "::",
    ] {
        if t.starts_with(kw) || t.contains(kw) {
            return true;
        }
    }
    false
}

/// Stable snake_case name for a content kind, used in the CCR store and in
/// HTTP responses.
pub fn kind_slug(kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::JsonArray => "json_array",
        ContentKind::JsonObject => "json_object",
        ContentKind::SourceCode => "source_code",
        ContentKind::BuildLog => "build_log",
        ContentKind::GitDiff => "git_diff",
        ContentKind::PlainText => "plain_text",
        ContentKind::Unknown => "unknown",
    }
}

/// Very rough token estimate (≈ 4 chars per token). Good enough for ratio math.
pub fn estimate_tokens(text: &str) -> usize {
    text.len().max(1) / 4
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
