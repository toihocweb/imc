//! Port of `main/proxy/lib/activity_extract.py`.
//!
//! Accumulates Anthropic SSE `content_block_*` events across one response
//! stream and publishes `tool_use` / `tool_result` events to
//! [`crate::activity_broker`]. Deliberately tolerant: every parse failure is
//! swallowed so activity capture never affects the proxied response.

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use crate::activity_broker;

/// Port of `activity_extract._norm_input`.
pub fn norm_input(input: Option<&Value>) -> Value {
    match input {
        Some(Value::Object(map)) => Value::Object(map.clone()),
        Some(Value::String(s)) => {
            if s.trim().is_empty() {
                return json!({});
            }
            match serde_json::from_str::<Value>(s) {
                Ok(parsed) => parsed,
                Err(_) => json!({ "_raw": s }),
            }
        }
        _ => json!({}),
    }
}

/// Port of `activity_extract.publish_tool_results_from_payload` — scans the
/// newest user message for `tool_result` blocks and publishes them.
pub fn publish_tool_results_from_payload(messages: &[Value]) {
    for msg in messages.iter().rev() {
        if msg.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let Some(Value::Array(content)) = msg.get("content") else {
            return;
        };
        for block in content {
            let Some(obj) = block.as_object() else {
                continue;
            };
            if obj.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let out = obj.get("content");
            let text = match out {
                Some(Value::Array(parts)) => {
                    let joined: Vec<String> = parts
                        .iter()
                        .filter(|o| o.get("type").and_then(Value::as_str) == Some("text"))
                        .map(|o| o.get("text").and_then(Value::as_str).unwrap_or("").to_string())
                        .collect();
                    Value::String(joined.join("\n"))
                }
                Some(v) => v.clone(),
                None => Value::Null,
            };
            let output = match text {
                Value::String(s) => s,
                other => serde_json::to_string(&other).unwrap_or_default(),
            };
            activity_broker::publish(&json!({
                "kind": "tool_result",
                "tool_use_id": obj.get("tool_use_id").and_then(Value::as_str).unwrap_or(""),
                "is_error": obj.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                "output": output,
            }));
        }
        return;
    }
}

/// Port of the `StreamActivityExtractor` class.
pub struct StreamActivityExtractor {
    /// index -> { name, id, input_json, input }
    blocks: HashMap<i64, StreamBlock>,
}

#[derive(Debug, Clone, Default)]
struct StreamBlock {
    name: String,
    id: String,
    input_json: String,
    input: Value,
}

impl Default for StreamActivityExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamActivityExtractor {
    /// Port of `StreamActivityExtractor.__init__`.
    pub fn new() -> Self {
        Self {
            blocks: HashMap::new(),
        }
    }

    /// Port of `feed_data_obj` — swallows errors like the Python wrapper.
    pub fn feed_data_obj(&mut self, data_obj: &Value) {
        let _ = self.feed(data_obj);
    }

    /// Port of `StreamActivityExtractor._feed`.
    pub fn feed(&mut self, data_obj: &Value) -> anyhow::Result<()> {
        let d_type = data_obj.get("type").and_then(Value::as_str).unwrap_or("");
        let idx = data_obj.get("index").and_then(Value::as_i64).unwrap_or(0);
        match d_type {
            "content_block_start" => {
                let block = data_obj.get("content_block");
                if block.and_then(|b| b.get("type")).and_then(Value::as_str) == Some("tool_use") {
                    let block = block.cloned().unwrap_or(Value::Null);
                    self.blocks.insert(
                        idx,
                        StreamBlock {
                            name: block
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                            id: block
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                            input_json: String::new(),
                            input: block
                                .get("input")
                                .filter(|v| !v.is_null())
                                .cloned()
                                .unwrap_or(json!({})),
                        },
                    );
                }
            }
            "content_block_delta" => {
                let delta = data_obj.get("delta");
                if delta.and_then(|d| d.get("type")).and_then(Value::as_str)
                    == Some("input_json_delta")
                {
                    if let Some(blk) = self.blocks.get_mut(&idx) {
                        if let Some(partial) = delta
                            .and_then(|d| d.get("partial_json"))
                            .and_then(Value::as_str)
                        {
                            blk.input_json.push_str(partial);
                        }
                    }
                }
            }
            "content_block_stop" => {
                if let Some(blk) = self.blocks.remove(&idx) {
                    let inp = if blk.input_json.is_empty() {
                        blk.input.clone()
                    } else {
                        Value::String(blk.input_json.clone())
                    };
                    activity_broker::publish(&json!({
                        "kind": "tool_use",
                        "id": blk.id,
                        "name": blk.name,
                        "input": norm_input(Some(&inp)),
                    }));
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Port of `flush` — publish any block that never got a stop event.
    pub fn flush(&mut self) {
        let drained: Vec<StreamBlock> = self.blocks.drain().map(|(_, v)| v).collect();
        for blk in drained {
            let raw = if blk.input_json.is_empty() {
                blk.input.clone()
            } else {
                Value::String(blk.input_json.clone())
            };
            activity_broker::publish(&json!({
                "kind": "tool_use",
                "id": blk.id,
                "name": blk.name,
                "input": norm_input(Some(&raw)),
            }));
        }
    }
}

/// Port of `publish_tool_uses_from_content` — non-stream responses.
pub fn publish_tool_uses_from_content(content_blocks: &[Value]) {
    for block in content_blocks {
        let Some(obj) = block.as_object() else {
            continue;
        };
        if obj.get("type").and_then(Value::as_str) == Some("tool_use") {
            let input = obj.get("input").cloned();
            activity_broker::publish(&json!({
                "kind": "tool_use",
                "id": obj.get("id").and_then(Value::as_str).unwrap_or(""),
                "name": obj.get("name").and_then(Value::as_str).unwrap_or(""),
                "input": norm_input(input.as_ref()),
            }));
        }
    }
}

/// Helper mirroring the `getattr(block, "__dict__", {})` fallback: pull an
/// object-ish view of a content block (either a JSON object or a struct that
/// serializes to one).
pub fn block_to_map(block: &Value) -> Map<String, Value> {
    block.as_object().cloned().unwrap_or_default()
}