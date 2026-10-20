//! Repair tool ids before the payload goes upstream.
//!
//! Anthropic validates every tool id against `^[a-zA-Z0-9_-]+$` and answers
//! `400` otherwise:
//!
//! ```text
//! messages.0.content.3.tool_result.tool_use_id: String should match pattern
//! '^[a-zA-Z0-9_-]+$'
//! ```
//!
//! A single character outside that class (a dot in a provider id such as
//! `call_abc.def`, a colon, a slash) makes the whole request fail, and the
//! conversation is stuck until the client is restarted.
//!
//! The fix has to keep the *pairing*: a `tool_result` refers to the `tool_use`
//! with the same id, and the API rejects a result without its call just as
//! hard. So every id is rewritten with the same rule everywhere it appears —
//! assistant `tool_use.id` and user `tool_result.tool_use_id` alike.

use std::collections::HashMap;

use imc_proxy_types::types::anthropic::{
    AnthropicAssistantContentBlock, AnthropicInputMessage, AnthropicMessagesPayload,
    AnthropicUserContentBlock, StringOrBlocks,
};

/// The pattern Anthropic enforces on tool ids.
pub const VALID_ID_CHARS: fn(char) -> bool = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';

/// Rewrite one id into something the API accepts.
///
/// The transformation is deterministic and applied to every occurrence of the
/// same id, so pairs stay together.
pub fn clean_id(id: &str) -> String {
    let mut out: String = id
        .chars()
        .map(|c| if VALID_ID_CHARS(c) { c } else { '_' })
        .collect();
    if out.is_empty() {
        out.push_str("imc_tool_id");
    }
    out
}

/// True when the id already satisfies the API's pattern.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(VALID_ID_CHARS)
}

/// How many ids were rewritten, and how many `tool_result` blocks had no
/// matching `tool_use` in the conversation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SanitizeReport {
    pub fixed_ids: usize,
    pub orphans: usize,
}

/// Repair every tool id in the payload.
///
/// Returns the number of distinct ids that were rewritten.
pub fn sanitize_tool_ids(payload: &mut AnthropicMessagesPayload) -> SanitizeReport {
    // 1. Collect the ids that appear anywhere, so one mapping serves both
    //    sides of a pair.
    let mut seen: Vec<String> = Vec::new();
    for message in &payload.messages {
        for id in ids_of(message) {
            if !seen.contains(&id) {
                seen.push(id);
            }
        }
    }

    let mut mapping: HashMap<String, String> = HashMap::new();
    for id in &seen {
        if !is_valid_id(id) {
            mapping.insert(id.clone(), clean_id(id));
        }
    }

    // 2. Rewrite both sides.
    for message in payload.messages.iter_mut() {
        rewrite_message(message, &mapping);
    }

    // 3. Count the results whose `tool_use` is nowhere in the conversation.
    let known: Vec<String> = payload
        .messages
        .iter()
        .flat_map(|m| tool_use_ids(m))
        .collect();
    let mut orphans = 0usize;
    for message in &payload.messages {
        for id in tool_result_ids(message) {
            if !known.contains(&id) {
                orphans += 1;
            }
        }
    }

    SanitizeReport {
        fixed_ids: mapping.len(),
        orphans,
    }
}

fn ids_of(message: &AnthropicInputMessage) -> Vec<String> {
    let mut out = tool_use_ids(message);
    out.extend(tool_result_ids(message));
    out
}

fn tool_use_ids(message: &AnthropicInputMessage) -> Vec<String> {
    let AnthropicInputMessage::Assistant(a) = message else {
        return Vec::new();
    };
    let StringOrBlocks::Blocks(blocks) = &a.content else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter_map(|b| match b {
            AnthropicAssistantContentBlock::ToolUse(t) => Some(t.id.clone()),
            _ => None,
        })
        .collect()
}

fn tool_result_ids(message: &AnthropicInputMessage) -> Vec<String> {
    let AnthropicInputMessage::User(u) = message else {
        return Vec::new();
    };
    let StringOrBlocks::Blocks(blocks) = &u.content else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter_map(|b| match b {
            AnthropicUserContentBlock::ToolResult(t) => Some(t.tool_use_id.clone()),
            _ => None,
        })
        .collect()
}

fn rewrite_message(message: &mut AnthropicInputMessage, mapping: &HashMap<String, String>) {
    match message {
        AnthropicInputMessage::Assistant(a) => {
            if let StringOrBlocks::Blocks(blocks) = &mut a.content {
                for block in blocks.iter_mut() {
                    if let AnthropicAssistantContentBlock::ToolUse(t) = block {
                        if let Some(new_id) = mapping.get(&t.id) {
                            t.id = new_id.clone();
                        }
                    }
                }
            }
        }
        AnthropicInputMessage::User(u) => {
            if let StringOrBlocks::Blocks(blocks) = &mut u.content {
                for block in blocks.iter_mut() {
                    if let AnthropicUserContentBlock::ToolResult(t) = block {
                        if let Some(new_id) = mapping.get(&t.tool_use_id) {
                            t.tool_use_id = new_id.clone();
                        }
                    }
                }
            }
        }
        AnthropicInputMessage::System(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imc_proxy_types::types::anthropic::{
        AnthropicAssistantMessage, AnthropicToolResultBlock, AnthropicToolUseBlock,
        AnthropicUserMessage,
    };

    fn payload_with(call_id: &str, result_id: &str) -> AnthropicMessagesPayload {
        let mut payload = AnthropicMessagesPayload {
            model: "m".into(),
            ..Default::default()
        };
        payload.messages.push(AnthropicInputMessage::Assistant(AnthropicAssistantMessage {
            role: "assistant".into(),
            content: StringOrBlocks::Blocks(vec![AnthropicAssistantContentBlock::ToolUse(
                AnthropicToolUseBlock {
                    r#type: "tool_use".into(),
                    id: call_id.into(),
                    name: "Read".into(),
                    input: Default::default(),
                    cache_control: None,
                },
            )]),
        }));
        payload.messages.push(AnthropicInputMessage::User(AnthropicUserMessage {
            role: "user".into(),
            content: StringOrBlocks::Blocks(vec![AnthropicUserContentBlock::ToolResult(
                AnthropicToolResultBlock {
                    r#type: "tool_result".into(),
                    tool_use_id: result_id.into(),
                    content: StringOrBlocks::Text("out".into()),
                    is_error: None,
                    cache_control: None,
                },
            )]),
        }));
        payload
    }

    #[test]
    fn valid_ids_are_left_alone() {
        let mut payload = payload_with("toolu_01ABC", "toolu_01ABC");
        let report = sanitize_tool_ids(&mut payload);
        assert_eq!(report.fixed_ids, 0);
        assert_eq!(report.orphans, 0);
        assert_eq!(tool_use_ids(&payload.messages[0])[0], "toolu_01ABC");
    }

    #[test]
    fn invalid_id_is_rewritten_on_both_sides() {
        let mut payload = payload_with("call_abc.def", "call_abc.def");
        let report = sanitize_tool_ids(&mut payload);
        assert_eq!(report.fixed_ids, 1);
        let call = tool_use_ids(&payload.messages[0])[0].clone();
        let result = tool_result_ids(&payload.messages[1])[0].clone();
        assert!(is_valid_id(&call), "{call}");
        assert!(is_valid_id(&result), "{result}");
        // The pair survives the rewrite.
        assert_eq!(call, result);
    }

    #[test]
    fn empty_id_gets_a_placeholder() {
        let mut payload = payload_with("", "");
        let report = sanitize_tool_ids(&mut payload);
        assert_eq!(report.fixed_ids, 1);
        let call = tool_use_ids(&payload.messages[0])[0].clone();
        assert!(is_valid_id(&call), "{call}");
        assert_eq!(call, tool_result_ids(&payload.messages[1])[0]);
    }

    #[test]
    fn orphan_result_is_counted() {
        let mut payload = payload_with("toolu_1", "toolu_missing");
        let report = sanitize_tool_ids(&mut payload);
        assert_eq!(report.fixed_ids, 0);
        assert_eq!(report.orphans, 1);
    }
}
