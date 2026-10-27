//! Dependency-light, provenance-preserving packing for generation context.
//! Faithful port of `main/search/context.py`.
//!
//! The budget is approximate: ceil(UTF-8 bytes / 3), not provider
//! tokenization. It includes file headers, AST annotations, separators and
//! truncation notices.

use std::collections::{HashMap, HashSet};

pub const DEFAULT_CONTEXT_TOKEN_BUDGET: i64 = 3000;
pub const TRUNCATION_MARKER: &str =
    "\n[Context truncated: approximate token budget reached; final source range may be partial.]\n";

/// Metadata a retrieval result carries (the Rust equivalent of the Python
/// `result.metadata` dict plus node char offsets).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContextMeta {
    pub file_path: Option<String>,
    pub start_line: Option<i64>,
    pub line_start: Option<i64>,
    pub end_line: Option<i64>,
    pub line_end: Option<i64>,
    pub start_char: Option<i64>,
    pub end_char: Option<i64>,
    pub node_start_char_idx: Option<i64>,
    pub node_end_char_idx: Option<i64>,
}

/// A retrieval result handed to [`pack_context`].
#[derive(Debug, Clone, Default)]
pub struct ContextResult {
    /// Main text (`result.get_text()` in Python).
    pub text: String,
    /// Optional override (`result.source_text`).
    pub source_text: Option<String>,
    /// AST annotations (`result.ast_context`).
    pub ast_context: String,
    pub meta: ContextMeta,
}

impl ContextResult {
    pub fn new(file_path: &str, text: &str, start_line: i64, end_line: i64) -> Self {
        Self {
            text: text.to_string(),
            source_text: None,
            ast_context: String::new(),
            meta: ContextMeta {
                file_path: Some(file_path.to_string()),
                start_line: Some(start_line),
                end_line: Some(end_line),
                ..Default::default()
            },
        }
    }

    pub fn with_ast_context(mut self, annotations: &str) -> Self {
        self.ast_context = annotations.to_string();
        self
    }
}

/// `ceil(len(text.encode("utf-8")) / 3)`.
pub fn estimate_tokens(text: &str) -> i64 {
    let b = text.len() as i64;
    (b + 2) / 3
}

/// Preserve stored node offsets without importing a retrieval framework.
/// Merges `node_start_char_idx`/`node_end_char_idx` under `start_char`/`end_char`
/// using `dict.setdefault` semantics.
fn result_metadata(r: &ContextResult) -> ContextMeta {
    let mut m = r.meta.clone();
    if m.start_char.is_none() {
        m.start_char = m.node_start_char_idx;
    }
    if m.end_char.is_none() {
        m.end_char = m.node_end_char_idx;
    }
    m
}

/// Return an inclusive, one-based line range only when explicitly supplied.
fn source_range(m: &ContextMeta) -> Option<(i64, i64)> {
    let start = m.start_line.or(m.line_start);
    let end = m.end_line.or(m.line_end);
    match (start, end) {
        (Some(s), Some(e)) if 0 < s && s <= e => Some((s, e)),
        _ => None,
    }
}

fn character_range(m: &ContextMeta) -> Option<(i64, i64)> {
    match (m.start_char, m.end_char) {
        (Some(s), Some(e)) if 0 <= s && s < e => Some((s, e)),
        _ => None,
    }
}

/// `str.splitlines(keepends=True)` — Python line splitting that retains
/// terminators (`\n`, `\r\n`, `\r`, `\x0b`, `\x0c`, `\x1c`-`\x1e`, `\x85`,
/// `\u2028`, `\u2029`).
fn is_line_boundary(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}'
            | '\u{2029}'
    )
}

pub(crate) fn splitlines_keepends(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if is_line_boundary(c) {
            cur.push(c);
            if c == '\r' {
                if let Some(&n) = it.peek() {
                    if n == '\n' {
                        cur.push(n);
                        it.next();
                    }
                }
            }
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `str.splitlines()` — Python line splitting that drops terminators.
pub(crate) fn splitlines(s: &str) -> Vec<String> {
    splitlines_keepends(s)
        .into_iter()
        .map(|mut line| {
            if line.ends_with('\n') {
                line.pop();
                if line.ends_with('\r') {
                    line.pop();
                }
            } else if line
                .chars()
                .last()
                .map(is_line_boundary)
                .unwrap_or(false)
            {
                line.pop();
            }
            line
        })
        .collect()
}

/// `text[from..to]` using Python's character (code point) indices.
fn char_slice(text: &str, from: i64, to: i64) -> String {
    text.chars().skip(from as usize).take((to - from) as usize).collect()
}

/// Keep retrieval order, remove proven overlap, and bound generation context.
///
/// Without ranges only exact same-file chunks and exact suffix/prefix line
/// overlaps are removed. Unknown paths are never conflated. Raw results remain
/// untouched; this API is only for context sent to generation.
pub fn pack_context(results: &[ContextResult], token_budget: i64) -> String {
    if token_budget <= 0 {
        return String::new();
    }

    let mut sections: Vec<String> = Vec::new();
    let mut covered: HashMap<String, HashSet<i64>> = HashMap::new();
    let mut previous: HashMap<String, Vec<String>> = HashMap::new();
    let mut exact: HashSet<(String, String, Option<(i64, i64)>, Option<(i64, i64)>)> = HashSet::new();
    let mut seen_annotations: HashSet<(String, String)> = HashSet::new();
    let mut character_coverage: HashMap<String, Vec<(i64, i64)>> = HashMap::new();

    for result in results {
        let metadata = result_metadata(result);
        let path = metadata
            .file_path
            .clone()
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| "Unknown".to_string());
        let mut text = match &result.source_text {
            Some(s) => s.clone(),
            None => result.text.clone(),
        };
        let mut annotations = result.ast_context.clone();

        if text.is_empty() && annotations.is_empty() {
            continue;
        }

        let known_path = path != "Unknown";
        let mut span = source_range(&metadata);
        let mut chars = character_range(&metadata);
        let key = (path.clone(), text.clone(), span, chars);

        if known_path {
            if seen_annotations.contains(&(path.clone(), annotations.clone())) {
                annotations = String::new();
            } else if !annotations.is_empty() {
                seen_annotations.insert((path.clone(), annotations.clone()));
            }
            if exact.contains(&key) {
                text.clear();
                span = None;
                chars = None;
            } else {
                exact.insert(key);
            }
        }

        let mut lines = splitlines_keepends(&text);

        if known_path && span.is_some() && lines.len() == (span.unwrap().1 - span.unwrap().0 + 1) as usize {
            let (s0, _e0) = span.unwrap();
            let seen = covered.entry(path.clone()).or_default();
            let mut runs: Vec<(i64, i64, String)> = Vec::new();
            for (idx, line) in lines.iter().enumerate() {
                let number = s0 + idx as i64;
                if !seen.contains(&number) {
                    let mut merged = false;
                    if let Some(last) = runs.last_mut() {
                        if last.1 + 1 == number {
                            last.1 = number;
                            last.2.push_str(line);
                            merged = true;
                        }
                    }
                    if !merged {
                        runs.push((number, number, line.clone()));
                    }
                    seen.insert(number);
                }
            }
            for (start, end, content) in runs {
                sections.push(format!(
                    "\n--- File: {path} (lines {start}-{end}) ---\n{}\n",
                    content.trim_end()
                ));
            }
            if !annotations.is_empty() {
                sections.push(format!("\n--- AST: {path} ---\n{annotations}\n"));
            }
        } else if known_path && chars.is_some() && text.chars().count() as i64 == (chars.unwrap().1 - chars.unwrap().0) {
            let base = chars.unwrap().0;
            let c_range = chars.unwrap();
            let intervals = character_coverage.entry(path.clone()).or_default();
            let mut runs: Vec<(i64, i64)> = vec![c_range];
            for &(old_start, old_end) in intervals.clone().iter() {
                let mut remaining: Vec<(i64, i64)> = Vec::new();
                for &(start, end) in &runs {
                    if old_end <= start || old_start >= end {
                        remaining.push((start, end));
                    } else {
                        if start < old_start {
                            remaining.push((start, old_start));
                        }
                        if old_end < end {
                            remaining.push((old_end, end));
                        }
                    }
                }
                runs = remaining;
            }
            intervals.push(c_range);
            for (start, end) in runs {
                let content = char_slice(&text, start - base, end - base);
                sections.push(format!(
                    "\n--- File: {path} (chars {start}-{end}, end-exclusive) ---\n{}\n",
                    content.trim_end()
                ));
            }
            if !annotations.is_empty() {
                sections.push(format!("\n--- AST: {path} ---\n{annotations}\n"));
            }
        } else {
            if known_path && !text.is_empty() && span.is_none() && chars.is_none() {
                let prior = previous.get(&path).cloned().unwrap_or_default();
                let min_len = std::cmp::min(prior.len(), lines.len());
                let mut trimmed = false;
                for size in (2..=min_len).rev() {
                    if prior[prior.len() - size..] == lines[..size] {
                        let joined: String = lines[..size].iter().cloned().collect();
                        if joined.len() >= 40 {
                            lines = lines[size..].to_vec();
                            trimmed = true;
                            break;
                        }
                    }
                }
                let _ = trimmed;
                previous.insert(path.clone(), splitlines_keepends(&text));
            }
            let content: String = lines.concat();
            if !content.is_empty() || !annotations.is_empty() {
                let mut section = format!("\n--- File: {path} ---\n{}", content.trim_end());
                if !annotations.is_empty() {
                    section.push_str(&format!("\n\n{annotations}"));
                }
                section.push('\n');
                sections.push(section);
            }
        }
    }

    let context = sections.concat();
    if estimate_tokens(&context) <= token_budget {
        return context;
    }
    if estimate_tokens(TRUNCATION_MARKER) > token_budget {
        // A tiny budget still produces an explicit notice, never unlabelled code.
        let notice = "[truncated]";
        return if estimate_tokens(notice) <= token_budget {
            notice.to_string()
        } else {
            String::new()
        };
    }
    let remaining = token_budget - estimate_tokens(TRUNCATION_MARKER);
    // Prefix clipping preserves retrieval order and leaves each source header
    // in front of its code. UTF-8 byte accounting never splits a Unicode char.
    let bytes = context.as_bytes();
    let n = std::cmp::min(remaining * 3, bytes.len() as i64) as usize;
    let slice = &bytes[..n];
    let prefix = match std::str::from_utf8(slice) {
        Ok(s) => s.to_string(),
        Err(e) => std::str::from_utf8(&slice[..e.valid_up_to()])
            .unwrap_or("")
            .to_string(),
    };
    // Do not leave a partial provenance header at the boundary.
    let mut prefix = prefix;
    if let Some(last_header) = prefix.rfind("\n--- ") {
        if !prefix[last_header..].contains(" ---\n") {
            prefix = prefix[..last_header].to_string();
        }
    }
    format!("{prefix}{TRUNCATION_MARKER}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(file: &str, text: &str, start: Option<i64>, end: Option<i64>) -> ContextResult {
        ContextResult {
            text: text.to_string(),
            source_text: None,
            ast_context: String::new(),
            meta: ContextMeta {
                file_path: Some(file.to_string()),
                start_line: start,
                end_line: end,
                ..Default::default()
            },
        }
    }

    #[test]
    fn estimate_tokens_ceil_div3() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 1);
        assert_eq!(estimate_tokens("abc"), 1);
        assert_eq!(estimate_tokens("abcd"), 2);
        // Multi-byte characters count as UTF-8 bytes.
        assert_eq!(estimate_tokens("é"), 1); // 2 bytes
        assert_eq!(estimate_tokens("€"), 1); // 3 bytes
        assert_eq!(estimate_tokens("€€"), 2); // 6 bytes
    }

    #[test]
    fn source_range_requires_explicit_valid_range() {
        let m = ContextMeta {
            start_line: Some(5),
            end_line: Some(10),
            ..Default::default()
        };
        assert_eq!(source_range(&m), Some((5, 10)));

        // start > end is invalid.
        let m = ContextMeta { start_line: Some(10), end_line: Some(5), ..Default::default() };
        assert_eq!(source_range(&m), None);

        // Missing end.
        let m = ContextMeta { start_line: Some(5), end_line: None, ..Default::default() };
        assert_eq!(source_range(&m), None);

        // Fallback keys line_start/line_end.
        let m = ContextMeta { line_start: Some(2), line_end: Some(3), ..Default::default() };
        assert_eq!(source_range(&m), Some((2, 3)));
    }

    #[test]
    fn splitlines_keepends_matches_python() {
        assert_eq!(splitlines_keepends("a\nb\n"), vec!["a\n", "b\n"]);
        assert_eq!(splitlines_keepends("a\nb"), vec!["a\n", "b"]);
        assert_eq!(splitlines_keepends("a\r\nb"), vec!["a\r\n", "b"]);
        assert_eq!(splitlines_keepends("a\rb"), vec!["a\r", "b"]);
        assert_eq!(splitlines_keepends(""), Vec::<String>::new());
    }

    #[test]
    fn splitlines_drops_terminators() {
        assert_eq!(splitlines("a\nb\n"), vec!["a", "b"]);
        assert_eq!(splitlines("a\r\nb"), vec!["a", "b"]);
        assert_eq!(splitlines("a"), vec!["a"]);
    }

    #[test]
    fn budget_zero_returns_empty() {
        let results = vec![result("a.rs", "fn a() {}", None, None)];
        assert_eq!(pack_context(&results, 0), "");
        assert_eq!(pack_context(&results, -1), "");
    }

    #[test]
    fn line_dedup_preserves_new_lines() {
        // Two overlapping ranges in the same file: only new lines are kept.
        let a = result("a.rs", "fn a() {\n    x\n}\n", Some(1), Some(3));
        let b = result("a.rs", "    x\n}\nfn b() {}\n", Some(2), Some(4));
        let out = pack_context(&[a, b], DEFAULT_CONTEXT_TOKEN_BUDGET);
        // First result emits all 3 lines; second only emits the new line 4.
        assert!(out.contains("(lines 1-3)"));
        assert!(out.contains("(lines 4-4)"));
        assert!(!out.contains("(lines 1-2)"));
    }

    #[test]
    fn exact_duplicate_chunk_dropped() {
        let a = result("a.rs", "fn a() {}", Some(1), Some(1));
        let b = result("a.rs", "fn a() {}", Some(1), Some(1));
        let out = pack_context(&[a, b], DEFAULT_CONTEXT_TOKEN_BUDGET);
        assert_eq!(out.matches("--- File: a.rs").count(), 1);
    }

    #[test]
    fn unknown_paths_never_conflated() {
        // Same text but Unknown path keeps both (no range => fallback branch).
        let mut a = result("x", "fn same() {}", None, None);
        a.meta.file_path = None;
        let mut b = result("y", "fn same() {}", None, None);
        b.meta.file_path = None;
        let out = pack_context(&[a, b], DEFAULT_CONTEXT_TOKEN_BUDGET);
        assert!(out.contains("Unknown"));
    }

    #[test]
    fn ast_annotations_emitted_once_per_path() {
        let a = result("a.rs", "line one here", Some(1), Some(1))
            .with_ast_context("sym: foo");
        let b = result("a.rs", "line two here", Some(2), Some(2))
            .with_ast_context("sym: foo");
        let out = pack_context(&[a, b], DEFAULT_CONTEXT_TOKEN_BUDGET);
        // The duplicate annotation (same path + text) is suppressed for b.
        assert_eq!(out.matches("--- AST: a.rs ---").count(), 1);
    }

    #[test]
    fn tiny_budget_yields_truncation_notice() {
        let mut big = String::new();
        for i in 0..50 {
            big.push_str(&format!("fn f{i}() {{ /* padding padding padding */ }}\n"));
        }
        let results = vec![result("a.rs", &big, None, None)];
        // Budget that fits the marker but not the content.
        let out = pack_context(&results, 30);
        assert!(out.ends_with(TRUNCATION_MARKER) || out == "[truncated]");
    }

    #[test]
    fn suffix_prefix_overlap_trimmed() {
        // Two chunks, same path, no line range: exact >=40 char suffix/prefix
        // overlap is removed from the second chunk. Chunk `a` must END with the
        // overlap and chunk `b` must START with the same lines.
        let first = "fn first() {}\nlet a = 1;\nlet b = 2;\nlet c = 3;\n";
        let overlap = "let x = 12345;\ndo_work(x);\nlet y = 67890;\nreturn;\n";
        let a = result("a.rs", &format!("{first}{overlap}"), None, None);
        let b = result("a.rs", &format!("{overlap}fn second() {{}}\n"), None, None);
        let out = pack_context(&[a, b], DEFAULT_CONTEXT_TOKEN_BUDGET);
        // The overlap body should appear only once (only chunk `a` keeps it).
        assert_eq!(out.matches("do_work").count(), 1);
        // Chunk b still contributes its non-overlapping remainder.
        assert!(out.contains("fn second()"));
    }
}
