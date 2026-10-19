//! Plain-text compression — removes redundant sentences while keeping
//! first/last sentences and keyword-dense ones. Used when no more specific
//! compressor applies.

use std::collections::HashMap;
use std::collections::HashSet;

pub fn compress(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return content.to_string();
    }
    let sentences: Vec<String> = split_sentences(trimmed);
    if sentences.len() <= 3 {
        return content.to_string();
    }
    let keep_first = 1;
    let keep_last = 1;
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
}
