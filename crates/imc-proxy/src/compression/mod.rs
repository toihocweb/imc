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

    // A log is a log when most of its lines are log records — or when it
    // carries a failure at all. A build that printed one ERROR between 80
    // lines of noise is a build that failed, and that ERROR is the answer;
    // classifying it as prose sent it to the sentence compressor, where a
    // line without a full stop is not a sentence and survives only by luck.
    let has_failure = lines.iter().any(|l| {
        let l = l.to_lowercase();
        l.contains("error") || l.contains("fail") || l.contains("panic")
    });
    if (log_lines * 2 >= total && log_lines > code_lines) || (has_failure && log_lines > 0) {
        return ContentKind::BuildLog;
    }
    // Source code needs the *majority* of the lines to look like code, not
    // one of them. A single "if " in a paragraph of prose was enough to
    // hand the whole document to the code compressor, which only collapses
    // repeated blocks — so prose came back uncompressed, and the wrong
    // compressor was the one that saw it.
    let meaningful = lines.iter().filter(|l| !l.trim().is_empty()).count().max(1);
    if code_lines * 2 >= meaningful && code_lines > 0 {
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
    if is_test_status_line(line) {
        return true;
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
/// A test runner's per-test line: a path, then a status marker at the end
/// ("tests/test_api.py::test_login PASSED").
///
/// Neither half looks like a log record on its own — the status is at the
/// end, not the start — so a whole pytest run was classified as plain text
/// and never compressed. The path also contains "::", which used to make the
/// line look like code, so the log and the code count cancelled out.
fn is_test_status_line(line: &str) -> bool {
    let t = line.trim_start();
    let Some(first) = t.split_whitespace().next() else {
        return false;
    };
    if !(first.contains('/') || first.contains('.')) || first.contains('=') {
        return false;
    }
    let last = t.split_whitespace().last().unwrap_or("").to_uppercase();
    matches!(
        last.as_str(),
        "PASSED" | "PASS" | "OK" | "SUCCESS" | "FAILED" | "FAIL" | "ERROR" | "SKIPPED" | "XFAIL"
            | "XPASS"
    )
}

fn is_code_line(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    // A per-test status line is a log record, not code, even though its path
    // contains "::".
    if is_test_status_line(line) {
        return false;
    }
    if t.starts_with("//") || t.starts_with('#') || t.starts_with("/*") || t.starts_with('*') {
        return true;
    }
    // A search result or a compiler diagnostic: "path/to/file.rs:42:  ...".
    // It ends in a semicolon and looks like a statement, but it is a *line
    // of output*, and a list of those is a list of matches — every line
    // carries different information, so the code compressor (which only
    // collapses repeated blocks) leaves it alone and the payload goes to
    // the model whole. A grep result over 150 files is one of the most
    // common tool results there is.
    if is_path_line_numbered(t) {
        return false;
    }
    if t.ends_with(';') || t.ends_with('{') || t.ends_with('}') || t == "}" || t == "{" {
        return true;
    }
    // Keywords are matched at the *start* of the line, not anywhere in it.
    // A substring match read English as code: "returns a response for each
    // case" contains "for ", "from " and "use ", and a document full of such
    // sentences was classified as source code — and then never compressed,
    // because the code compressor only collapses repeated blocks.
    for kw in [
        "fn ", "let ", "pub ", "impl ", "use ", "mod ", "struct ", "enum ", "trait ",
        "match ", "if ", "else ", "for ", "while ", "return ", "def ", "class ",
        "import ", "from ", "const ", "var ", "function ",
    ] {
        if t.starts_with(kw) {
            return true;
        }
    }
    // The structural operators are different: they are attached to the
    // code around them, so they can appear after a type or an argument
    // list ("    cb: fn(i32) -> i32,"). "::" and "=>" are rare enough in
    // prose to be trusted anywhere in the line; "->" is not, so it only
    // counts next to a type signature.
    if t.contains("::") || t.contains("=>") {
        return true;
    }
    if t.contains("->") && t.contains("fn ") {
        return true;
    }
    false
}

/// True for a line that starts with a path followed by a line number:
/// `src/main.rs:42:  let x = 1;` — a search result, a compiler diagnostic,
/// a stack frame. The path may not contain a colon, so `http://x` is not
/// one.
fn is_path_line_numbered(t: &str) -> bool {
    let Some(colon) = t.find(':') else {
        return false;
    };
    let path = &t[..colon];
    if path.is_empty() || path.contains(' ') || path.contains("://") {
        return false;
    }
    let rest = &t[colon + 1..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    !digits.is_empty() && rest[digits.len()..].starts_with(':')
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


    /// A pytest run prints one line per test. The status is at the *end* of
    /// the line, so neither half looked like a log record, and the path
    /// contains "::", which made the line look like code. The whole run was
    /// classified as source code and came back untouched.
    #[test]
    fn a_test_run_is_a_build_log() {
        let mut log = String::new();
        for i in 0..40 {
            log.push_str(&format!("tests/test_mod_{}.py::test_case_{i} PASSED\n", i / 20));
        }
        log.push_str("tests/test_a.py::test_bad FAILED\n");
        log.push_str("40 passed, 1 failed in 3.2s\n");

        assert_eq!(detect_kind(&log), ContentKind::BuildLog);

        // The failure and the summary survive the compression.
        let out = log_compressor::compress(&log);
        assert!(out.contains("FAILED"), "{out}");
        assert!(out.contains("40 passed, 1 failed"), "{out}");
        assert!(out.len() * 10 < log.len(), "{} vs {}", out.len(), log.len());
    }

    /// A per-test status line is a log record, not code — the "::" in the
    /// path must not make it look like a Rust path.
    #[test]
    fn a_test_status_line_is_not_code() {
        assert!(!is_code_line("tests/test_api.py::test_login PASSED"));
        assert!(is_log_line("tests/test_api.py::test_login PASSED"));
        // Real code is still code.
        assert!(is_code_line("fn main() {"));
    }

    /// English prose is not source code.
    ///
    /// The keyword test used to look for the word *anywhere* in the line,
    /// and English is full of words that are also keywords: "returns a
    /// response for each case" contains "for ", "comes from the cache"
    /// contains "from ", "we use the helper" contains "use ". A document
    /// written in English was classified as source code, and the code
    /// compressor only collapses repeated blocks — so the document came
    /// back uncompressed, having been handed to the wrong compressor.
    #[test]
    fn prose_is_not_source_code() {
        let prose = (0..40)
            .map(|i| {
                format!(
                    "The system processes the request and returns a response for case {i}.\n"
                )
            })
            .collect::<String>();
        assert_eq!(detect_kind(&prose), ContentKind::PlainText);

        // Real code is still code.
        let code = "fn main() {\n    let x = 1;\n    if x > 0 { return x; }\n}\n".repeat(5);
        assert_eq!(detect_kind(&code), ContentKind::SourceCode);
    }

    /// A search result is a list of matches, not code.
    ///
    /// Every line names a different file and a different line number, so
    /// the code compressor — which collapses repeated *blocks* — leaves the
    /// whole thing alone. A grep over 150 files is one of the most common
    /// tool results there is.
    #[test]
    fn a_search_result_is_not_source_code() {
        let grep = (0..40)
            .map(|i| format!("crates/foo/src/bar_{i}.rs:42:  let x = compute({i});\n"))
            .collect::<String>();
        assert_eq!(detect_kind(&grep), ContentKind::PlainText);
        assert!(!is_code_line("crates/foo/src/bar_0.rs:42:  let x = compute(0);"));
        // A URL is not a path with a line number.
        assert!(!is_path_line_numbered("see http://example.com:8080/docs"));
    }

    /// A build that printed one failure between 80 lines of noise is a
    /// build that failed, and that failure is the answer.
    #[test]
    fn a_log_with_one_failure_is_a_build_log() {
        let mut log = String::new();
        for i in 0..40 {
            log.push_str(&format!("line {i} noise\n"));
        }
        log.push_str("ERROR: the database migration failed\n");
        for i in 0..40 {
            log.push_str(&format!("line {i} noise\n"));
        }
        assert_eq!(detect_kind(&log), ContentKind::BuildLog);

        let out = log_compressor::compress(&log);
        assert!(out.contains("ERROR: the database migration failed"), "{out}");
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
