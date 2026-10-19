//! Git diff compression — structural, hunk-aware.
//!
//! Diffs are already dense, so the win is not "drop lines" but "don't send the
//! same hunk twice". Generated diffs (lockfiles, snapshots, repeated fixtures)
//! contain the same file section many times over. We:
//!   1. split the diff into per-file sections (`diff --git ...`),
//!   2. keep the first occurrence of each identical section and replace later
//!      duplicates with a one-line marker,
//!   3. inside a section, keep every `+`/`-` line (that is the signal) and
//!      trim long runs of unchanged context lines down to a couple.
//!
//! Nothing that changed is ever dropped.

/// Context lines kept at each end of a long unchanged run. Git's default of
/// three is for a human scrolling a terminal; two is enough to place a change,
/// and a run of three is left alone entirely (trimming it would just add a
/// marker for no gain).
const CONTEXT_EDGE: usize = 2;
/// An unchanged run longer than this gets trimmed.
const CONTEXT_TRIM_AFTER: usize = 5;

pub fn compress(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= 6 {
        return content.to_string();
    }

    // `git show` prepends the commit message. It is metadata, not diff, and on
    // a merge or a long body it can be a third of the payload. Keep the subject
    // line, elide the body.
    let (preamble, diff_start) = split_preamble(&lines);
    let mut out: Vec<String> = preamble;

    let sections = split_sections(&lines[diff_start..]);
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for section in sections {
        let body = trim_context(&section);
        let fingerprint = fingerprint(&body);
        if let Some(first_seen) = seen.get(&fingerprint).copied() {
            let header = section
                .first()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "diff".to_string());
            out.push(format!(
                "{} … [identical to the section at line {}; {} lines omitted] …",
                header,
                first_seen,
                body.len()
            ));
            continue;
        }
        seen.insert(fingerprint, out.len());
        out.extend(body);
    }

    if out.is_empty() {
        content.to_string()
    } else {
        out.join("\n")
    }
}

/// Split a `git show`-style payload into (kept preamble, index where the diff
/// starts). A plain `git diff` has no preamble, so the index is 0.
fn split_preamble(lines: &[&str]) -> (Vec<String>, usize) {
    let Some(diff_at) = lines.iter().position(|l| l.starts_with("diff --git ")) else {
        return (Vec::new(), 0);
    };
    if diff_at == 0 {
        return (Vec::new(), 0);
    }
    let head = &lines[..diff_at];
    // Subject: the first indented non-empty line after the `Author:`/`Date:`
    // block, or the first non-empty line if this is not a commit header.
    let mut kept: Vec<String> = Vec::new();
    for line in head {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.starts_with("commit ") || t.starts_with("Author:") || t.starts_with("Date:") {
            kept.push(line.to_string());
            continue;
        }
        // First real message line = subject; everything after it is body.
        kept.push(line.to_string());
        let body_lines = head.len().saturating_sub(kept.len());
        if body_lines > 2 {
            kept.push(format!("… [{body_lines} commit-message lines omitted] …"));
        }
        break;
    }
    (kept, diff_at)
}

/// Split the diff into per-file sections, each starting at a `diff --git` line.
fn split_sections(lines: &[&str]) -> Vec<Vec<String>> {
    let mut sections: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for line in lines {
        if line.starts_with("diff --git") && !current.is_empty() {
            sections.push(std::mem::take(&mut current));
        }
        current.push(line.to_string());
    }
    if !current.is_empty() {
        sections.push(current);
    }
    sections
}

/// Trim long runs of unchanged (context) lines and drop the `index <hash>`
/// bookkeeping line. Changed lines are always kept.
fn trim_context(section: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(section.len());
    let mut i = 0;
    while i < section.len() {
        let line = &section[i];
        // Blob hashes are for git's object store, not for a reader.
        if line.starts_with("index ") {
            i += 1;
            continue;
        }
        let is_context = line.starts_with(' ')
            || line.starts_with("new file mode")
            || line.starts_with("deleted file mode");
        if !is_context {
            out.push(line.clone());
            i += 1;
            continue;
        }
        // Measure the run of unchanged lines starting here.
        let mut run = 1;
        while i + run < section.len() {
            let next = &section[i + run];
            let next_context = next.starts_with(' ')
                || next.starts_with("new file mode")
                || next.starts_with("deleted file mode");
            if next_context {
                run += 1;
            } else {
                break;
            }
        }
        if run > CONTEXT_TRIM_AFTER {
            out.extend_from_slice(&section[i..i + CONTEXT_EDGE]);
            out.push(format!("… [{} unchanged lines] …", run - 2 * CONTEXT_EDGE));
            out.extend_from_slice(&section[i + run - CONTEXT_EDGE..i + run]);
        } else {
            out.extend_from_slice(&section[i..i + run]);
        }
        i += run;
    }
    out
}

/// Content fingerprint of a section: its changed lines only, so two sections
/// that differ solely in context still count as the same hunk.
fn fingerprint(body: &[String]) -> String {
    let mut sig = String::new();
    for line in body {
        if line.starts_with('+') || line.starts_with('-') {
            sig.push_str(line);
            sig.push('\n');
        }
    }
    if sig.is_empty() {
        sig.push_str(&body.join("\n"));
    }
    sig
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = "diff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,5 +1,5 @@\n fn main() {\n-    println!(\"hello world\");\n+    println!(\"hello imc\");\n     return();\n }\n";

    #[test]
    fn elides_commit_message_body_but_keeps_subject() {
        let mut input = String::from(
            "commit abc123\nAuthor: a <a@b.c>\nDate:   Mon Oct 19 06:07:15 2026 +0700\n\n    subject line here\n\n    body paragraph one\n    body paragraph two\n    body paragraph three\n",
        );
        input.push_str(ONE);
        let out = compress(&input);
        assert!(out.contains("subject line here"), "subject must survive: {out}");
        assert!(!out.contains("body paragraph two"), "body must go: {out}");
        assert!(out.contains("commit-message lines omitted"), "must announce: {out}");
    }

    #[test]
    fn drops_index_hash_lines() {
        let input = "diff --git a/x b/x\nindex 1234567..89abcde 100644\n--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n";
        let out = compress(input);
        assert!(!out.contains("index 1234567"), "blob hash is noise: {out}");
        assert!(out.contains("+b"));
    }

    #[test]
    fn preserves_hunks() {
        let out = compress(ONE);
        assert!(out.contains("diff --git"));
        assert!(out.contains("--- a/src/main.rs"));
        assert!(out.contains("+++ b/src/main.rs"));
        assert!(out.contains("@@ -1,5 +1,5 @@"));
        assert!(out.contains("-    println!(\"hello world\");"));
        assert!(out.contains("+    println!(\"hello imc\");"));
    }

    #[test]
    fn dedupes_identical_file_sections() {
        let input = ONE.repeat(5);
        let out = compress(&input);
        assert!(out.len() < input.len(), "expected shrink");
        assert_eq!(out.matches("+    println!(\"hello imc\");").count(), 1);
        assert!(out.contains("identical to the section"));
    }

    #[test]
    fn keeps_distinct_sections() {
        let other = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,2 @@\n+pub mod net;\n";
        let input = format!("{ONE}{other}");
        let out = compress(&input);
        assert!(out.contains("src/main.rs"));
        assert!(out.contains("src/lib.rs"));
        assert!(out.contains("+pub mod net;"));
    }

    #[test]
    fn trims_long_context_runs() {
        let mut input = String::from("diff --git a/big.txt b/big.txt\n--- a/big.txt\n+++ b/big.txt\n@@ -1,40 +1,41 @@\n");
        for _ in 0..40 {
            input.push_str(" unchanged line\n");
        }
        input.push_str("+added line\n");
        let out = compress(&input);
        assert!(out.contains("+added line"));
        assert!(out.contains("unchanged lines"));
        assert!(out.len() < input.len());
    }

    #[test]
    fn short_diff_unchanged() {
        let input = "diff --git a/foo.rs b/foo.rs\n";
        assert_eq!(compress(input), input);
    }
}
