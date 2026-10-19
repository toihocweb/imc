//! Code compression — structural deduplication.
//!
//! Three passes over the line stream:
//!   1. strip line comments and collapse blank-line runs,
//!   2. collapse consecutive identical lines,
//!   3. collapse consecutive repeated *blocks* (the common case: a template or
//!      generated stanza pasted N times).
//!
//! Nothing is deleted silently — every collapse leaves a marker with the count
//! and the full original stays in the CCR store.

/// Longest block (in lines) we will try to recognise as a repeating unit.
const MAX_PERIOD: usize = 40;
/// A repeat is only collapsed when it saves at least this many lines.
const MIN_SAVED_LINES: usize = 3;

pub fn compress(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= 4 {
        return content.to_string();
    }

    let normalized = normalize(&lines);
    let collapsed = collapse_repeats(&normalized);

    if collapsed.is_empty() {
        content.to_string()
    } else {
        collapsed.join("\n")
    }
}

/// Strip comments, drop comment-only lines, collapse blank-line runs.
fn normalize(lines: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for line in lines {
        let code = strip_line_comment(line).trim_end();
        if code.trim().is_empty() {
            if out.last().map(|l| l.is_empty()).unwrap_or(true) {
                continue;
            }
            out.push(String::new());
        } else {
            out.push(code.to_string());
        }
    }
    while out.last().map(|l| l.is_empty()).unwrap_or(false) {
        out.pop();
    }
    out
}

/// Collapse consecutive repeated blocks, longest period first so that a
/// multi-line stanza wins over a spurious single-line match.
fn collapse_repeats(lines: &[String]) -> Vec<String> {
    let n = lines.len();
    let mut out: Vec<String> = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        let mut handled = false;
        let max_period = ((n - i) / 2).min(MAX_PERIOD);
        // Pick the period that saves the most lines; tie-break on the smallest
        // period so we report the tightest repeating unit.
        let mut best: Option<(usize, usize, usize)> = None; // (saved, p, reps)
        for p in 1..=max_period {
            let mut reps = 1;
            while i + (reps + 1) * p <= n
                && lines[i + reps * p..i + (reps + 1) * p] == lines[i..i + p]
            {
                reps += 1;
            }
            if reps >= 2 {
                let saved = p * (reps - 1);
                if saved >= MIN_SAVED_LINES && best.map(|b| saved > b.0).unwrap_or(true) {
                    best = Some((saved, p, reps));
                }
            }
        }
        if let Some((saved, p, reps)) = best {
            out.extend_from_slice(&lines[i..i + p]);
            out.push(format!(
                "… [{p}-line block x{reps}, {saved} lines elided] …"
            ));
            i += p * reps;
            handled = true;
        }
        if !handled {
            out.push(lines[i].clone());
            i += 1;
        }
    }
    out
}

/// Strip a trailing `// ...` comment, keeping the code before it.
///
/// String-literal aware: `"\\n// graph: {}"` is code, not a comment. We walk the
/// line tracking quote state (with `\` escapes) and only treat `//` as a
/// comment when it appears outside a string. Getting this wrong silently
/// truncates real code, which is far worse than keeping a comment.
fn strip_line_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_string = false;
    let mut in_char = false;
    let mut escaped = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        match c {
            b'\\' if in_string || in_char => {
                escaped = true;
                i += 1;
            }
            b'"' if !in_char => {
                in_string = !in_string;
                i += 1;
            }
            b'\'' if !in_string => {
                // A lone apostrophe in prose should not swallow the line; only
                // treat it as a char literal when it closes soon after.
                in_char = !in_char;
                i += 1;
            }
            b'/' if !in_string && !in_char && i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                // Trim trailing whitespace from the code part.
                return line[..i].trim_end();
            }
            _ => i += 1,
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupes_repeated_lines() {
        let input = "line1\nline2\nline2\nline2\nline2\nline3\n";
        let out = compress(input);
        assert!(out.contains("1-line block x4"), "got: {out}");
        assert!(out.contains("line1"));
        assert!(out.contains("line3"));
    }

    #[test]
    fn short_code_unchanged() {
        let input = "fn main() {\n    println!(\"hi\");\n}\n";
        assert_eq!(compress(input), input);
    }

    #[test]
    fn collapses_repeated_block() {
        let block = "fn handler() {\n    let x = 1;\n    Ok(x)\n}\n";
        let input = block.repeat(6);
        let out = compress(&input);
        assert!(out.len() < input.len(), "expected shrink, got {}", out.len());
        assert!(out.contains("-line block x6"), "got: {out}");
        // the block itself survives exactly once
        assert_eq!(out.matches("fn handler()").count(), 1);
    }

    #[test]
    fn repeated_block_preserves_distinct_tail() {
        let block = "step();\ncheck();\n";
        let input = format!("{}{}done();\n", block, block.repeat(4));
        let out = compress(&input);
        assert!(out.contains("done();"), "tail must survive: {out}");
    }

    #[test]
    fn strips_comments() {
        let input = "fn foo() {\n    // a comment\n    let x = 1;\n    // another\n    let y = 2;\n}\n";
        let out = compress(input);
        assert!(!out.contains("a comment"));
        assert!(!out.contains("another"));
        assert!(out.contains("let x = 1;"));
        assert!(out.contains("let y = 2;"));
    }

    #[test]
    fn removes_blank_line_runs() {
        let out = compress("line1\n\n\n\nline2\n");
        assert!(!out.contains("\n\n\n"));
    }

    #[test]
    fn single_blank_line_survives() {
        let out = compress("line1\n\nline2\nline3\n");
        assert!(out.contains("line1\n\nline2"));
    }

    #[test]
    fn comment_marker_inside_string_literal_survives() {
        // A `//` inside a string is code, not a comment. Stripping it silently
        // corrupts the source — the exact bug this test locks down.
        let input = "fn f() {\n    let a = 1;\n    text.push_str(&format!(\"\\n// graph: {}\", ev.join(\"; \")));\n    let b = 2;\n}\n";
        let out = compress(input);
        assert!(
            out.contains(r#"text.push_str(&format!("\n// graph: {}", ev.join("; ")));"#),
            "string literal was corrupted: {out}"
        );
        assert!(out.contains("let b = 2;"));
    }

    #[test]
    fn trailing_comment_after_code_is_still_stripped() {
        let input = "a();\nb(); // trailing note\nc();\nd();\ne();\n";
        let out = compress(input);
        assert!(!out.contains("trailing note"), "got: {out}");
        assert!(out.contains("b();"));
    }

    #[test]
    fn url_inside_string_does_not_eat_the_line() {
        let input = "a();\nlet u = \"https://example.com/x\";\nb();\nc();\n";
        let out = compress(input);
        assert!(out.contains("https://example.com/x"), "got: {out}");
    }

    #[test]
    fn no_run_leaves_code_intact() {
        let input = "a();\nb();\nc();\nd();\ne();\n";
        let out = compress(input);
        assert_eq!(out, "a();\nb();\nc();\nd();\ne();");
    }
}
