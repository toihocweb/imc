//! Plain-text compression — removes redundant sentences while keeping
//! first/last sentences and keyword-dense ones. Used when no more specific
//! compressor applies.

use std::collections::HashMap;
use std::collections::HashSet;

use crate::compression::context::is_search_or_discovery_content;

pub fn compress(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return content.to_string();
    }
    // Search hits, grep lines, or directory listings must never drop items
    if is_search_or_discovery_content(trimmed) {
        return content.to_string();
    }
    let sentences: Vec<String> = split_sentences(trimmed);
    if sentences.len() <= 3 {
        // No sentence structure. A file listing, a grep result, a table —
        // many lines, none of them a sentence. The sentence-based pass
        // would see one long "sentence" and leave the text alone, so the
        // line-based pass gets its turn.
        return compress_by_lines(content);
    }
    let mut keep: HashSet<usize> = HashSet::new();
    keep.insert(0);
    keep.insert(sentences.len() - 1);

    // Score each sentence by keyword density (rare words = high signal).
    let word_freq = word_frequencies(&sentences);
    let mut scored: Vec<(usize, f64)> = sentences
        .iter()
        .enumerate()
        .filter(|(i, _)| !keep.contains(i))
        .map(|(i, s)| {
            let score: f64 = s
                .split_whitespace()
                .filter_map(|w| word_freq.get(w))
                .map(|f| 1.0 / (f + 1) as f64)
                .sum();
            (i, score / s.split_whitespace().count().max(1) as f64)
        })
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Keep top ~30% of middle sentences.
    let keep_middle = ((sentences.len() - 2) as f64 * 0.30).round() as usize;
    for (i, _) in scored.iter().take(keep_middle) {
        keep.insert(*i);
    }

    let mut out = String::new();
    for (i, s) in sentences.iter().enumerate() {
        if keep.contains(&i) {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(s);
        } else if i > 0 && i < sentences.len() - 1 {
            if !out.ends_with(" …") {
                out.push_str(" …");
            }
        }
    }
    if out.is_empty() {
        content.to_string()
    } else {
        out
    }
}

/// Line-based compression for content that has no sentence structure.
///
/// A file listing, a grep result, a table: one item per line, none of them
/// a sentence. The sentence-based pass sees a single long "sentence" and
/// leaves the text alone, so the whole payload went to the model untouched.
///
/// The lines that carry the signal stay: the first, the last, and any line
/// that looks like a failure. The routine middle is dropped, and the drop
/// is announced — an agent that wants the full list has to be able to tell
/// that the list is incomplete.
fn compress_by_lines(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= 12 {
        return content.to_string();
    }
    let keep_first = 2;
    let keep_last = 2;
    let mut keep: Vec<bool> = vec![false; lines.len()];
    for i in 0..keep_first.min(lines.len()) {
        keep[i] = true;
    }
    for i in (lines.len() - keep_last)..lines.len() {
        keep[i] = true;
    }
    for (i, line) in lines.iter().enumerate() {
        let l = line.to_lowercase();
        if l.contains("error")
            || l.contains("fail")
            || l.contains("panic")
            || l.contains("exception")
            || l.contains("fatal")
            || l.contains("denied")
            || l.contains("not found")
            || l.contains("warning")
        {
            keep[i] = true;
        }
    }

    let mut out = String::new();
    let mut elided = 0usize;
    for (i, line) in lines.iter().enumerate() {
        if keep[i] {
            if elided > 0 {
                out.push_str(&format!("… [{} lines omitted] …\n", elided));
                elided = 0;
            }
            out.push_str(line);
            out.push('\n');
        } else {
            elided += 1;
        }
    }
    if elided > 0 {
        out.push_str(&format!("… [{} lines omitted] …", elided));
    }
    if out.is_empty() {
        content.to_string()
    } else {
        out
    }
}

/// Split text into sentences on ., !, ? followed by whitespace or end.
fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    for (i, c) in chars.iter().enumerate() {
        current.push(*c);
        if matches!(c, '.' | '!' | '?') {
            let next_is_space = i + 1 < n && chars[i + 1].is_whitespace();
            let is_end = i + 1 == n;
            if next_is_space || is_end {
                let s = current.trim().to_string();
                if !s.is_empty() {
                    out.push(s);
                }
                current = String::new();
            }
        }
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_string());
    }
    out
}

/// Word frequencies across sentences (lower = rarer = higher signal).
fn word_frequencies(sentences: &[String]) -> HashMap<String, usize> {
    let mut freq: HashMap<String, usize> = HashMap::new();
    for s in sentences {
        for w in s.split_whitespace() {
            let w = w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
            if w.len() > 2 {
                *freq.entry(w).or_insert(0) += 1;
            }
        }
    }
    freq
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_first_and_last() {
        let input = "First sentence. Middle one. Middle two. Last sentence.";
        let out = compress(input);
        assert!(out.contains("First sentence"));
        assert!(out.contains("Last sentence"));
    }

    #[test]
    fn short_text_unchanged() {
        let input = "Just one sentence.";
        assert_eq!(compress(input), input);
    }

    /// A file listing has no sentence structure: one path per line, not one
    /// of them ending in a full stop. The sentence-based pass saw a single
    /// long "sentence" and left the list alone, so the whole payload went
    /// to the model untouched.
    #[test]
    fn a_file_listing_is_compressed() {
        let listing = (0..200)
            .map(|i| format!("src/module_{i}/handler.rs\n"))
            .collect::<String>();
        let out = compress(&listing);
        assert!(out.len() * 10 < listing.len(), "{} vs {}", out.len(), listing.len());
        // The elision is announced — an agent has to be able to tell that
        // the list is incomplete.
        assert!(out.contains("lines omitted"), "{out}");
    }

    /// The line that says what went wrong stays, whatever else is dropped.
    #[test]
    fn a_failure_line_survives_the_line_compression() {
        let mut listing = (0..100)
            .map(|i| format!("src/module_{i}/handler.rs\n"))
            .collect::<String>();
        listing.push_str("src/broken.rs:10: ERROR: file not found\n");
        listing.push_str(
            &(100..200)
                .map(|i| format!("src/module_{i}/handler.rs\n"))
                .collect::<String>(),
        );
        let out = compress(&listing);
        assert!(out.contains("ERROR: file not found"), "{out}");
    }

    /// The first and the last line stay: the head of a list says what the
    /// list is, the tail says where it ended.
    #[test]
    fn the_edges_of_a_listing_survive() {
        let listing = (0..100)
            .map(|i| format!("entry_{i}\n"))
            .collect::<String>();
        let out = compress(&listing);
        assert!(out.contains("entry_0"), "{out}");
        assert!(out.contains("entry_99"), "{out}");
    }

    #[test]
    fn search_hits_are_not_compressed() {
        let input = "1. [chunk] src/auth.rs:10-30 auth_handler (score 0.91) fn login() {}\n\
                     2. [chunk] src/user.rs:20-40 get_user (score 0.88) fn user() {}\n\
                     3. [chunk] src/db.rs:5-25 query_db (score 0.72) fn db() {}\n\
                     4. [chunk] src/session.rs:1-15 session (score 0.65) fn sess() {}";
        let out = compress(input);
        assert_eq!(out, input);
    }
}
