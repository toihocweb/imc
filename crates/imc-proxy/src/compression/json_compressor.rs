//! JSON array compression — port of Headroom's statistical array compressor.
//!
//! Tool outputs are usually JSON arrays of results. We keep error items,
//! anomalies (values outside the normal statistical range), and first/last
//! boundaries, dropping the routine middle. Signal survives aggressive
//! compression because it is selected statistically, not by keyword.

use serde_json::Value;

/// Compress a JSON array of tool results. Keeps:
///   * every object with an error-like field,
///   * every object whose numeric values are statistical anomalies,
///   * the first N and last N items (boundary preservation),
///   * everything else is dropped with a placeholder note.
pub fn compress(content: &str) -> String {
    let arr: Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => return content.to_string(),
    };
    let Value::Array(items) = arr else {
        return content.to_string();
    };
    if items.is_empty() {
        return content.to_string();
    }

    let n = items.len();
    let keep_first = 2.min(n);
    let keep_last = 2.min(n.saturating_sub(keep_first));
    let mut keep: HashMap<usize, bool> = HashMap::new();
    for i in 0..keep_first {
        keep.insert(i, true);
    }
    for i in (n - keep_last)..n {
        keep.insert(i, true);
    }

    // Mark error-like items.
    for (i, item) in items.iter().enumerate() {
        if is_error_like(item) {
            keep.insert(i, true);
        }
    }

    // Mark anomaly items (numeric outliers).
    let numeric_items: Vec<(usize, f64)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| extract_number(item).map(|v| (i, v)))
        .collect();
    let anomalies = find_anomalies(&numeric_items);
    for i in anomalies {
        keep.insert(i, true);
    }

    let mut out = Vec::new();
    out.push("[".to_string());
    let mut first = true;
    for (i, item) in items.iter().enumerate() {
        if keep.get(&i).copied().unwrap_or(false) {
            if !first {
                out.push(", ".to_string());
            }
            out.push(serde_json::to_string(item).unwrap_or_default());
            first = false;
        } else if i > keep_first && i < n.saturating_sub(keep_last) {
            // Dropped middle — note it once per contiguous run.
            if first || out.last().map(|s| s.as_str()) != Some("... [items omitted] ...") {
                if !first {
                    out.push(", ".to_string());
                }
                out.push(r#"... [items omitted] ..."#.to_string());
                first = false;
            }
        }
    }
    out.push("]".to_string());
    out.join("")
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
            // Look for common numeric fields: duration, size, count, tokens, etc.
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

use std::collections::HashMap;

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
        assert!(out.contains("1"));
        assert!(out.contains("8"));
    }

    #[test]
    fn empty_array_unchanged() {
        let input = "[]";
        assert_eq!(compress(input), input);
    }

    #[test]
    fn non_array_unchanged() {
        let input = r#"{"a":1}"#;
        assert_eq!(compress(input), input);
    }
}
