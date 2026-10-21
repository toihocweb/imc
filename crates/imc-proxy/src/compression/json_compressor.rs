//! JSON array compression — statistical array compressor.
//!
//! Two independent wins, applied in this order:
//!
//!   1. **Run collapse** — consecutive identical items become one item plus a
//!      count. This is lossless in the sense that the reader can reconstruct
//!      exactly what was there, and it is where the big ratios come from
//!      (generated fixtures, repeated API rows, duplicated tool output).
//!   2. **Statistical selection** — *only* for genuinely large arrays do we
//!      drop the routine middle, keeping error items, numeric anomalies, and
//!      the first/last boundary items.
//!
//! The selection step is deliberately gated: a short array of 20 distinct
//! search hits is real information and gets returned intact. Dropping most of
//! a small result set to post a nicer ratio is how a compressor quietly
//! destroys the agent's context.

use serde_json::Value;
use std::collections::HashMap;

/// Arrays shorter than this are never sampled down — only run-collapsed.
const SAMPLE_THRESHOLD: usize = 40;
/// Boundary items kept when sampling a large array.
const KEEP_FIRST: usize = 3;
const KEEP_LAST: usize = 3;

/// Compress a JSON array or object of tool results.
///   * runs of identical items collapse to one item + a count marker,
///   * arrays of at least `SAMPLE_THRESHOLD` groups additionally keep error
///     items, numeric anomalies, and first/last boundaries, dropping the
///     routine middle.
///   * objects with embedded arrays or oversized string fields are compressed structurally.
pub fn compress(content: &str) -> String {
    let parsed: Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => return content.to_string(),
    };
    match parsed {
        Value::Array(items) => compress_array(&items),
        Value::Object(map) => compress_object(map, content),
        _ => content.to_string(),
    }
}

fn compress_object(mut map: serde_json::Map<String, Value>, original: &str) -> String {
    let mut modified = false;
    for (_k, v) in map.iter_mut() {
        match v {
            Value::Array(arr) if arr.len() >= 6 => {
                let compressed_arr_str = compress_array(arr);
                if let Ok(Value::Array(new_arr)) = serde_json::from_str::<Value>(&compressed_arr_str) {
                    let old_len = serde_json::to_string(arr).unwrap_or_default().len();
                    if compressed_arr_str.len() + 10 < old_len {
                        *v = Value::Array(new_arr);
                        modified = true;
                    }
                }
            }
            Value::String(s) if s.chars().count() > MAX_FIELD_CHARS => {
                *v = Value::String(window_string(s));
                modified = true;
            }
            _ => {}
        }
    }
    if modified {
        serde_json::to_string(&Value::Object(map)).unwrap_or_else(|_| original.to_string())
    } else {
        original.to_string()
    }
}

fn compress_array(items: &[Value]) -> String {
    if items.is_empty() {
        return "[]".to_string();
    }

    // Pass 1 — collapse consecutive identical items into (representative, count).
    let mut groups: Vec<(Value, usize)> = Vec::new();
    for item in items {
        match groups.last_mut() {
            Some((last, count)) if *last == *item => *count += 1,
            _ => groups.push((item.clone(), 1)),
        }
    }

    let n = groups.len();
    // Sampling is opt-in: short arrays keep every distinct item. That is the
    // honest behaviour — 20 search hits are 20 pieces of information.
    let sampling = n >= SAMPLE_THRESHOLD;
    let keep_first = if sampling { KEEP_FIRST.min(n) } else { n };
    let keep_last = if sampling {
        KEEP_LAST.min(n.saturating_sub(keep_first))
    } else {
        0
    };
    let mut keep: Vec<bool> = vec![false; n];
    for slot in keep.iter_mut().take(keep_first) {
        *slot = true;
    }
    for slot in keep.iter_mut().skip(n - keep_last) {
        *slot = true;
    }

    // Error-like items are always kept.
    for (i, (item, _)) in groups.iter().enumerate() {
        if is_error_like(item) {
            keep[i] = true;
        }
    }

    // Numeric anomalies are always kept.
    let numeric: Vec<(usize, f64)> = groups
        .iter()
        .enumerate()
        .filter_map(|(i, (item, _))| extract_number(item).map(|v| (i, v)))
        .collect();
    for i in find_anomalies(&numeric) {
        keep[i] = true;
    }

    // Pass 2 — emit. Identical runs collapse to one item plus a count marker.
    let mut out = vec!["[".to_string()];
    let mut first = true;
    let mut marker_emitted = false;
    for (i, (item, count)) in groups.iter().enumerate() {
        if keep[i] || n <= keep_first + keep_last {
            if !first {
                out.push(", ".to_string());
            }
            let emitted = elide_redundant_fields(item);
            out.push(serde_json::to_string(&emitted).unwrap_or_default());
            first = false;
            marker_emitted = false;
            if *count > 1 {
                out.push(format!(
                    ", \"… [{} identical items omitted] …\"",
                    count - 1
                ));
            }
        } else if !marker_emitted {
            if !first {
                out.push(", ".to_string());
            }
            out.push(r#""… [items omitted] …""#.to_string());
            first = false;
            marker_emitted = true;
        }
    }
    out.push("]".to_string());
    out.join("")
}

/// String fields longer than this get windowed down to head+tail.
const MAX_FIELD_CHARS: usize = 1500;
/// Head/tail window kept when a field is windowed.
const FIELD_HEAD: usize = 900;
const FIELD_TAIL: usize = 300;

/// Drop object fields that are wholly contained in a sibling field, and window
/// down any single field that is larger than a whole result set needs.
///
/// Tool results routinely carry the same payload twice under different names
/// (`text` and `snippet`, `content` and `body`), and a search hit often embeds
/// an entire 20 KB symbol body to point at four relevant lines. Both are waste:
/// the reader needs the matched region, not the whole file, and the original is
/// in the CCR store either way. Every elision is announced in `_elided` so
/// nothing disappears silently.
fn elide_redundant_fields(item: &Value) -> Value {
    let Value::Object(map) = item else {
        return item.clone();
    };
    if map.len() < 2 {
        return item.clone();
    }

    // Collect the string fields, longest first. On a length tie, prefer to keep
    // the "primary" field name so `text` wins over `snippet`.
    let primary = |k: &str| -> u8 {
        match k {
            "text" | "content" | "body" | "message" | "msg" | "output" => 0,
            _ => 1,
        }
    };
    let mut strings: Vec<(&String, &String)> = map
        .iter()
        .filter_map(|(k, v)| match v {
            Value::String(s) if !s.is_empty() => Some((k, s)),
            _ => None,
        })
        .collect();
    if strings.len() < 2 {
        return item.clone();
    }
    strings.sort_by(|a, b| {
        b.1.len()
            .cmp(&a.1.len())
            .then_with(|| primary(a.0).cmp(&primary(b.0)))
            .then_with(|| a.0.cmp(b.0))
    });

    // After sorting, element i is the longest / most-primary field. Anything
    // after it that it fully contains is the duplicate, so drop the later one.
    let mut drop_keys: Vec<String> = Vec::new();
    for (i, (keep_key, keep_val)) in strings.iter().enumerate() {
        for (dup_key, dup_val) in strings.iter().skip(i + 1) {
            // Only near-duplicates: the duplicate must be most of the kept one.
            if dup_val.len() * 10 < keep_val.len() * 7 {
                continue;
            }
            if keep_val.contains(dup_val.as_str()) {
                drop_keys.push((*dup_key).clone());
                drop_keys.push(format!("{dup_key} (≈{keep_key})"));
                break;
            }
        }
    }
    // (windowing below runs regardless of whether a duplicate was found)

    let dropped: Vec<String> = drop_keys.chunks(2).map(|c| c[1].clone()).collect();
    let mut out = serde_json::Map::new();
    for (k, v) in map {
        if !drop_keys.chunks(2).any(|c| c[0] == *k) {
            out.insert(k.clone(), v.clone());
        }
    }

    // Window any remaining over-long string field. Announced, not silent.
    let mut notes: Vec<String> = Vec::new();
    for (k, v) in out.clone() {
        if let Value::String(s) = &v {
            if s.chars().count() > MAX_FIELD_CHARS {
                out.insert(k.clone(), Value::String(window_string(s)));
                notes.push(format!(
                    "{k}: windowed to {FIELD_HEAD}+{FIELD_TAIL} of {} chars (full text via retrieve)",
                    s.chars().count()
                ));
            }
        }
    }

    if !dropped.is_empty() {
        notes.insert(0, format!("duplicate of sibling: {}", dropped.join(", ")));
    }
    if !notes.is_empty() {
        out.insert("_elided".to_string(), Value::String(notes.join("; ")));
    }
    Value::Object(out)
}

/// Keep a head and tail slice of an over-long string, joined by a marker.
fn window_string(s: &str) -> String {
    let total = s.chars().count();
    let head: String = s.chars().take(FIELD_HEAD).collect();
    let tail: String = s
        .chars()
        .skip(total.saturating_sub(FIELD_TAIL))
        .collect();
    let elided = total.saturating_sub(FIELD_HEAD + FIELD_TAIL);
    format!("{head}\n… [{elided} chars omitted] …\n{tail}")
}

fn is_error_like(item: &Value) -> bool {
    match item {
        Value::Object(map) => {
            for (k, v) in map {
                let k_lower = k.to_lowercase();
                if k_lower.contains("error")
                    || k_lower.contains("fail")
                    || k_lower.contains("exception")
                    || k_lower.contains("panic")
                    || k_lower.contains("err")
                {
                    if matches!(v, Value::String(s) if !s.is_empty())
                        || matches!(v, Value::Bool(true))
                    {
                        return true;
                    }
                }
                if k_lower == "status" || k_lower == "code" {
                    if let Value::String(s) = v {
                        if !s.is_empty() && s != "200" && s != "ok" && s != "success" {
                            return true;
                        }
                    }
                    if let Value::Number(num) = v {
                        if num.is_i64() && num.as_i64().unwrap_or(0) != 0 && num.as_i64().unwrap_or(0) != 200
                        {
                            return true;
                        }
                    }
                }
            }
            false
        }
        _ => false,
    }
}

/// Extract a representative f64 from a JSON value for anomaly detection.
fn extract_number(item: &Value) -> Option<f64> {
    match item {
        Value::Number(n) => n.as_f64(),
        Value::Object(map) => {
            for k in ["duration", "size", "count", "tokens", "time", "ms", "bytes"] {
                if let Some(Value::Number(n)) = map.get(k) {
                    return n.as_f64();
                }
            }
            None
        }
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// Statistical anomaly detection: values more than 2 standard deviations from
/// the mean are anomalies.
fn find_anomalies(items: &[(usize, f64)]) -> Vec<usize> {
    if items.is_empty() {
        return Vec::new();
    }
    let mean = items.iter().map(|(_, v)| v).sum::<f64>() / items.len() as f64;
    let variance = items
        .iter()
        .map(|(_, v)| (v - mean).powi(2))
        .sum::<f64>()
        / items.len() as f64;
    let std = variance.sqrt();
    if std == 0.0 {
        return Vec::new();
    }
    let threshold = 2.0 * std;
    items
        .iter()
        .filter(|(_, v)| (v - mean).abs() > threshold)
        .map(|(i, _)| *i)
        .collect()
}

// Kept for callers that still build the old map-based keep set.
#[allow(dead_code)]
fn keep_map(n: usize) -> HashMap<usize, bool> {
    HashMap::with_capacity(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_errors() {
        let input = r#"[{"status":"ok"},{"status":"error","msg":"x"},{"status":"ok"}]"#;
        let out = compress(input);
        assert!(out.contains("error"));
        assert!(out.contains("x"));
    }

    #[test]
    fn keeps_first_and_last() {
        let input = r#"[1,2,3,4,5,6,7,8]"#;
        let out = compress(input);
        assert!(out.contains('1'));
        assert!(out.contains('8'));
    }

    #[test]
    fn collapses_identical_items() {
        let one = r#"{"tool":"read","file":"a.rs"}"#;
        let input = format!(
            "[{}]",
            (0..12).map(|_| one).collect::<Vec<_>>().join(",")
        );
        let out = compress(&input);
        assert!(out.len() < input.len() / 2, "expected big shrink: {out}");
        assert!(out.contains("11 identical items omitted"), "got: {out}");
        assert_eq!(out.matches(r#""tool":"read""#).count(), 1);
    }

    #[test]
    fn elides_duplicate_sibling_string_field() {
        let input = r#"[{"file":"a.rs","text":"fn main() { x(); }","snippet":"fn main() { x(); }"},{"file":"b.rs","text":"fn b() {}","snippet":"fn b() {}"}]"#;
        let out = compress(input);
        assert!(!out.contains(r#""snippet""#), "duplicate snippet must go: {out}");
        assert!(out.contains(r#""text":"fn main() { x(); }""#), "text must stay: {out}");
        assert!(out.contains("_elided"), "elision must be announced: {out}");
    }

    #[test]
    fn keeps_distinct_sibling_fields() {
        let input = r#"[{"file":"a.rs","text":"fn main() { x(); }","note":"totally different"}]"#;
        let out = compress(input);
        assert!(out.contains("note"), "distinct field must survive: {out}");
        assert!(out.contains("text"));
    }

    #[test]
    fn windows_over_long_field_and_announces_it() {
        let big = "x".repeat(5000);
        let input = format!(r#"[{{"file":"a.rs","text":"{big}"}}]"#);
        let out = compress(&input);
        assert!(out.len() < 1500, "expected windowing, got {} bytes", out.len());
        assert!(out.contains("chars omitted"), "elision must be announced: {}", &out[..200.min(out.len())]);
        assert!(out.contains(r#""file":"a.rs""#), "other fields must survive");
    }

    #[test]
    fn keeps_short_field_untouched() {
        let input = r#"[{"file":"a.rs","text":"short body"}]"#;
        let out = compress(input);
        assert!(out.contains("short body"));
    }

    #[test]
    fn short_distinct_array_unchanged() {
        // 20 distinct items — real information, must survive intact.
        let mut input = String::from("[");
        for i in 0..20 {
            if i > 0 {
                input.push(',');
            }
            input.push_str(&format!("{{\"id\":{i}}}"));
        }
        input.push(']');
        let out = compress(&input);
        let a: serde_json::Value = serde_json::from_str(&input).unwrap();
        let b: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(a, b, "short distinct array must not be sampled: {out}");
    }

    #[test]
    fn long_array_keeps_anomaly_and_marks_omission() {
        // 50 near-identical items, one outlier — outlier survives + marker.
        let mut items = Vec::new();
        for i in 0..49 {
            items.push(format!("{{\"d\":{i}}}"));
        }
        items.push(r#"{"d":900}"#.to_string());
        let input = format!("[{}]", items.join(","));
        let out = compress(&input);
        assert!(out.contains("900"), "anomaly must survive: {out}");
        assert!(out.contains("[items omitted"), "marker expected: {out}");
    }

    #[test]
    fn keeps_anomaly_among_collapsed_runs() {
        let input = r#"[{"d":1},{"d":1},{"d":1},{"d":1},{"d":1},{"d":1},{"d":1},{"d":1},{"d":1},{"d":900}]"#;
        let out = compress(input);
        assert!(out.contains("900"), "anomaly must survive: {out}");
    }

    #[test]
    fn empty_array_unchanged() {
        assert_eq!(compress("[]"), "[]");
    }

    #[test]
    fn non_array_unchanged() {
        let input = r#"{"a":1}"#;
        assert_eq!(compress(input), input);
    }

    #[test]
    fn compresses_object_with_embedded_array() {
        let one = r#"{"tool":"read","file":"a.rs"}"#;
        let items = (0..12).map(|_| one).collect::<Vec<_>>().join(",");
        let input = format!(r#"{{"status":"ok","items":[{items}]}}"#);
        let out = compress(&input);
        assert!(out.contains("11 identical items omitted"), "got: {out}");
        let val: serde_json::Value = serde_json::from_str(&out).expect("valid json output");
        assert_eq!(val["status"], "ok");
    }
}
