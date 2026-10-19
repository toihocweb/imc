//! Build/test log compression — keeps failures, errors, anomalies, and the
//! first/last lines. Passing noise is dropped.

pub fn compress(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return content.to_string();
    }
    if lines.len() <= 6 {
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

    // Error/failure lines.
    for (i, line) in lines.iter().enumerate() {
        let l = line.to_lowercase();
        if l.contains("error")
            || l.contains("fail")
            || l.contains("panic")
            || l.contains("exception")
            || l.contains("fatal")
            || l.contains("reject")
        {
            keep[i] = true;
        }
    }

    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if keep[i] {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(line);
        } else if i > keep_first && i < lines.len() - keep_last {
            if !out.ends_with("\n…") {
                out.push('\n');
                out.push_str("…");
            }
        }
    }
    if out.is_empty() {
        content.to_string()
    } else {
        out
    }
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
    fn short_log_unchanged() {
        let input = "a\nb\nc";
        assert_eq!(compress(input), input);
    }
}
