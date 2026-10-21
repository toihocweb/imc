//! Build/test log compression — keeps failures, errors, anomalies, stack traces,
//! diagnostics, and the first/last lines. Passing noise is dropped.

const MAX_DIAGNOSTIC_LOOKAHEAD: usize = 16;

pub fn compress(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= 6 {
        return content.to_string();
    }

    let keep_first = 2;
    let keep_last = 2;
    let mut keep: Vec<bool> = vec![false; lines.len()];
    for slot in keep.iter_mut().take(keep_first.min(lines.len())) {
        *slot = true;
    }
    for slot in keep.iter_mut().skip(lines.len().saturating_sub(keep_last)) {
        *slot = true;
    }

    // Pass 1: detect error / failure trigger lines and preserve their surrounding diagnostics
    for (i, line) in lines.iter().enumerate() {
        if is_error_trigger(line) {
            keep[i] = true;

            // Preserve preceding header if it introduced a traceback or failure section
            if i > 0 && is_failure_header(lines[i - 1]) {
                keep[i - 1] = true;
            }

            // Look forward and preserve diagnostic lines (file paths, gutter indicators,
            // stack traces, assertion messages, and indented source lines).
            let mut j = i + 1;
            let mut consecutive_blank = 0;
            while j < lines.len() && j < i + 1 + MAX_DIAGNOSTIC_LOOKAHEAD {
                let next_line = lines[j];
                if next_line.trim().is_empty() {
                    consecutive_blank += 1;
                    if consecutive_blank > 1 {
                        break;
                    }
                    j += 1;
                    continue;
                }
                consecutive_blank = 0;
                if is_diagnostic_continuation(next_line) || is_error_trigger(next_line) {
                    keep[j] = true;
                    j += 1;
                } else {
                    break;
                }
            }
        }
    }

    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if keep[i] {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(line);
        } else if i > keep_first && i < lines.len() - keep_last && !out.ends_with("\n…") {
            out.push('\n');
            out.push('…');
        }
    }
    if out.is_empty() {
        content.to_string()
    } else {
        out
    }
}

fn is_error_trigger(line: &str) -> bool {
    let l = line.to_lowercase();
    if l.contains("error")
        || l.contains("fail")
        || l.contains("panic")
        || l.contains("exception")
        || l.contains("fatal")
        || l.contains("reject")
    {
        return true;
    }
    // The rest are phrases, not loose words. A log that says
    // "request completed (timeout=30s configured)" or "container not killed
    // yet" is a log of things going *well*, and matching the single word
    // "timeout" or "killed" in it keeps every line of the log — which is
    // the same as not compressing it at all. A failure says "timed out",
    // "was killed", "assertion failed".
    l.contains("timed out")
        || l.contains("timeout exceeded")
        || l.contains("timeout error")
        || l.contains("deadline exceeded")
        || l.contains("segmentation fault")
        || l.contains("sigsegv")
        || l.contains("was killed")
        || l.contains("killed by")
        || l.contains("out of memory")
        || l.contains("oom killer")
        || l.contains("assertion failed")
        || l.contains("assertionerror")
        || l.contains("traceback (most recent call last)")
}

fn is_failure_header(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("===")
        || t.starts_with("---")
        || t.starts_with("___")
        || t.to_lowercase().contains("traceback")
        || t.to_lowercase().contains("failures")
}

fn is_diagnostic_continuation(line: &str) -> bool {
    let t = line.trim_start();
    if t.is_empty() {
        return false;
    }
    let is_indented = line.starts_with(' ') || line.starts_with('\t');
    let is_line_numbered_gutter = if let Some(pos) = t.find('|') {
        let prefix = t[..pos].trim();
        prefix.is_empty() || prefix.chars().all(|c| c.is_ascii_digit())
    } else {
        false
    };
    let is_gutter = t.starts_with("-->")
        || is_line_numbered_gutter
        || t.starts_with('^')
        || t.starts_with(":::")
        || t.starts_with("= note:")
        || t.starts_with("= help:")
        || t.starts_with("help:")
        || t.starts_with("note:")
        || t.starts_with("E ")
        || t.starts_with("> ");
    is_indented || is_gutter
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_errors() {
        let input = "ok line 1\nok line 2\nERROR something broke\nok line 3";
        let out = compress(input);
        assert!(out.contains("ERROR"));
    }

    #[test]
    fn preserves_compiler_diagnostic_context() {
        let input = "Compiling pkg v1.0.0\nChecking dep1 v1.0.0\nChecking dep2 v1.0.0\nChecking dep3 v1.0.0\nerror[E0308]: mismatched types\n  --> src/main.rs:12:5\n   |\n12 |     foo(bar)\n   |     ^^^ expected `u32`, found `&str`\nFor more information about this error, try `rustc --explain E0308`.\nFinished dev target";
        let out = compress(input);
        assert!(out.contains("error[E0308]: mismatched types"), "error header kept");
        assert!(out.contains("--> src/main.rs:12:5"), "location line kept");
        assert!(out.contains("foo(bar)"), "source line kept");
        assert!(out.contains("expected `u32`, found `&str`"), "explanation kept");
    }

    #[test]
    fn preserves_python_traceback_context() {
        let input = "Starting app\nConnecting db\nLoading config\nTraceback (most recent call last):\n  File \"app/main.py\", line 45, in run\n    res = fetch_data()\n  File \"app/utils.py\", line 12, in fetch_data\n    raise ValueError(\"Invalid key\")\nValueError: Invalid key\nCleanup done\nExited";
        let out = compress(input);
        assert!(out.contains("Traceback"), "header kept");
        assert!(out.contains("app/main.py"), "caller file kept");
        assert!(out.contains("app/utils.py"), "origin file kept");
        assert!(out.contains("ValueError: Invalid key"), "exception kept");
    }

    /// A word that can appear in a routine line must not keep the whole
    /// log. "timeout=30s configured" and "container not killed yet" are
    /// things going well, and a log full of them is exactly the log worth
    /// compressing.
    #[test]
    fn a_routine_line_is_not_a_failure() {
        let routine = (0..100)
            .map(|_| "2026-10-21 10:00:00 INFO request completed (timeout=30s configured)\n".to_string())
            .collect::<String>();
        let out = compress(&routine);
        assert!(out.len() * 10 < routine.len(), "{} vs {}", out.len(), routine.len());

        for phrase in ["not killed yet", "assertion passed", "no timeout occurred"] {
            let t = (0..100)
                .map(|_| format!("2026-10-21 10:00:00 INFO {phrase}\n"))
                .collect::<String>();
            let out = compress(&t);
            assert!(out.len() * 10 < t.len(), "{phrase}: {} vs {}", out.len(), t.len());
        }
    }

    /// The failures the new phrases cover still have to be kept.
    #[test]
    fn a_phrase_failure_is_still_a_failure() {
        for phrase in [
            "request timed out after 30s",
            "process was killed by the OOM killer",
            "assertion failed: left == right",
            "deadline exceeded",
        ] {
            let mut t = (0..50)
                .map(|i| format!("2026-10-21 10:00:00 INFO routine {i}\n"))
                .collect::<String>();
            t.push_str(&format!("2026-10-21 10:05:00 ERROR {phrase}\n"));
            let out = compress(&t);
            assert!(out.contains(phrase), "{phrase} was dropped: {out}");
        }
    }

    #[test]
    fn short_log_unchanged() {
        let input = "a\nb\nc";
        assert_eq!(compress(input), input);
    }
}
