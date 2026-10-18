//! Port of Python's `json.dumps` default serialization.
//!
//! `non_stream_translation` and `responses_translation` build the
//! `tool_calls[].function.arguments` / `function_call.arguments` strings with
//! `json.dumps(input)`. Those strings are handed back to the model as opaque
//! JSON, so byte-level fidelity matters more than compactness: Python emits
//! `", "` / `": "` separators and escapes non-ASCII under its default
//! `ensure_ascii=True`, and an agent that string-compares arguments should see
//! the same bytes it would have seen from the Python proxy.
//!
//! Anything `serde_json` cannot round-trip through this formatting (floats
//! whose shortest repr differs, e.g. `1e20`) is handed to `serde_json`'s
//! compact writer, which is still valid JSON.
//!
//! Known deviation: `serde_json::Map` is a `BTreeMap` here, so object keys come
//! out sorted, whereas a Python dict keeps the order the JSON arrived in.
//! That is semantically irrelevant — the model parses the arguments back into
//! a map — and enabling serde_json's `preserve_order` feature would change
//! every sibling crate's `Map` behaviour in the same build, so it is left
//! alone deliberately.

use serde_json::Value;

/// Serialize `value` the way Python's `json.dumps(value)` does by default.
pub fn python_json_dumps(value: &Value) -> String {
    let mut out = String::new();
    write_value(value, &mut out);
    out
}

fn write_value(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    // Python's default `separators` are (", ", ": ").
                    out.push_str(", ");
                }
                write_value(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (k, v)) in map.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_string(k, out);
                out.push_str(": ");
                write_value(v, out);
            }
            out.push('}');
        }
    }
}

/// Python's `json.dumps` string escaping under the default `ensure_ascii=True`:
/// the standard escapes plus `\uXXXX` for every non-ASCII code point.
fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 > 0x7e => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}