//! Port of `main/proxy/services/copilot/preprocess.py` — ephemeral
//! `cache_control` marker injection.
//!
//! Anthropic bills cache writes at a premium and caps a request at four
//! marked blocks. These three helpers each strip every existing marker
//! before placing exactly one, so a client that sends its own markers never
//! pushes the request over that cap.
//!
//! All three take `&mut` because Python mutates the payload in place.

use imc_proxy_types::types::anthropic::{
    AnthropicAssistantContentBlock, AnthropicCacheControl, AnthropicInputMessage,
    AnthropicMessagesPayload, AnthropicTextBlock, AnthropicUserContentBlock, StringOrBlocks,
};

use crate::non_stream_translation::{assistant_content_block_view, user_content_block_view};

/// The only cache-control value this proxy writes.
fn ephemeral() -> AnthropicCacheControl {
    AnthropicCacheControl {
        r#type: "ephemeral".to_string(),
        ..AnthropicCacheControl::default()
    }
}

/// Port of `apply_system_cache_control` — the system prompt is static across
/// turns, so it is the highest-value block to cache.
pub fn apply_system_cache_control(payload: &mut AnthropicMessagesPayload) {
    let Some(system) = payload.system.take() else {
        return;
    };

    match system {
        StringOrBlocks::Text(s) => {
            payload.system = Some(StringOrBlocks::Blocks(vec![AnthropicTextBlock {
                r#type: "text".to_string(),
                text: s,
                cache_control: Some(ephemeral()),
            }]));
        }
        StringOrBlocks::Blocks(mut blocks) => {
            if blocks.is_empty() {
                // `if isinstance(payload.system, list) and payload.system`
                payload.system = Some(StringOrBlocks::Blocks(blocks));
                return;
            }
            for block in blocks.iter_mut() {
                block.cache_control = None;
            }
            blocks.last_mut().expect("non-empty").cache_control = Some(ephemeral());
            payload.system = Some(StringOrBlocks::Blocks(blocks));
        }
    }
}

/// Port of `apply_tools_cache_control` — tool lists are static within a
/// session, so the last tool definition gets the marker.
pub fn apply_tools_cache_control(payload: &mut AnthropicMessagesPayload) {
    let Some(tools) = payload.tools.as_mut() else {
        return;
    };
    if tools.is_empty() {
        return;
    }

    for tool in tools.iter_mut() {
        if let imc_proxy_types::types::anthropic::AnthropicToolEntry::Tool(t) = tool {
            t.cache_control = None;
        }
    }

    if let Some(imc_proxy_types::types::anthropic::AnthropicToolEntry::Tool(last)) =
        tools.last_mut()
    {
        last.cache_control = Some(ephemeral());
    }
}

/// Port of `apply_last_message_cache_control` — mark the final user message so
/// the user query and its memories stay cached across turns.
///
/// If no user message exists, Python falls back to the last message of any
/// role; thinking blocks are never marked, because marking one is rejected
/// upstream.
pub fn apply_last_message_cache_control(payload: &mut AnthropicMessagesPayload) {
    if payload.messages.is_empty() {
        return;
    }

    // First, strip all existing cache_control blocks from all messages to
    // ensure we don't exceed the 4 block limit.
    for msg in payload.messages.iter_mut() {
        clear_message_cache_control(msg);
    }

    // Target the last user message to ensure the user query & memories are
    // cached.
    let last_user_index = payload
        .messages
        .iter()
        .rposition(|m| message_role(m) == "user")
        .unwrap_or(payload.messages.len() - 1);

    match &mut payload.messages[last_user_index] {
        // If the content is just a string, convert it to a list containing one
        // TextBlock.
        AnthropicInputMessage::User(m) => {
            if let StringOrBlocks::Text(s) = &m.content {
                m.content = StringOrBlocks::Blocks(vec![AnthropicUserContentBlock::Text(
                    AnthropicTextBlock {
                        r#type: "text".to_string(),
                        text: s.clone(),
                        cache_control: Some(ephemeral()),
                    },
                )]);
                return;
            }
            mark_last_user_block(&mut m.content);
        }
        AnthropicInputMessage::Assistant(m) => {
            if let StringOrBlocks::Text(s) = &m.content {
                m.content = StringOrBlocks::Blocks(vec![AnthropicAssistantContentBlock::Text(
                    AnthropicTextBlock {
                        r#type: "text".to_string(),
                        text: s.clone(),
                        cache_control: Some(ephemeral()),
                    },
                )]);
                return;
            }
            mark_last_assistant_block(&mut m.content);
        }
        AnthropicInputMessage::System(m) => {
            if let StringOrBlocks::Text(s) = &m.content {
                m.content = StringOrBlocks::Blocks(vec![AnthropicTextBlock {
                    r#type: "text".to_string(),
                    text: s.clone(),
                    cache_control: Some(ephemeral()),
                }]);
                return;
            }
            mark_last_system_block(&mut m.content);
        }
    }
}

/// The message's `role` attribute — what Python's `getattr(msg, "role", "")`
/// returns, independent of which untagged variant holds it.
fn message_role(msg: &AnthropicInputMessage) -> &str {
    match msg {
        AnthropicInputMessage::User(m) => &m.role,
        AnthropicInputMessage::Assistant(m) => &m.role,
        AnthropicInputMessage::System(m) => &m.role,
    }
}

fn clear_message_cache_control(msg: &mut AnthropicInputMessage) {
    match msg {
        AnthropicInputMessage::User(m) => {
            if let StringOrBlocks::Blocks(blocks) = &mut m.content {
                for block in blocks.iter_mut() {
                    match block {
                        imc_proxy_types::types::anthropic::AnthropicUserContentBlock::Text(b) => {
                            b.cache_control = None
                        }
                        imc_proxy_types::types::anthropic::AnthropicUserContentBlock::Image(b) => {
                            b.cache_control = None
                        }
                        imc_proxy_types::types::anthropic::AnthropicUserContentBlock::Document(
                            b,
                        ) => b.cache_control = None,
                        imc_proxy_types::types::anthropic::AnthropicUserContentBlock::ToolResult(
                            b,
                        ) => b.cache_control = None,
                    }
                }
            }
        }
        AnthropicInputMessage::Assistant(m) => {
            if let StringOrBlocks::Blocks(blocks) = &mut m.content {
                for block in blocks.iter_mut() {
                    match block {
                        imc_proxy_types::types::anthropic::AnthropicAssistantContentBlock::Text(b) => {
                            b.cache_control = None
                        }
                        imc_proxy_types::types::anthropic::AnthropicAssistantContentBlock::ToolUse(
                            b,
                        ) => b.cache_control = None,
                        imc_proxy_types::types::anthropic::AnthropicAssistantContentBlock::Thinking(
                            _,
                        ) => {}
                    }
                }
            }
        }
        AnthropicInputMessage::System(m) => {
            if let StringOrBlocks::Blocks(blocks) = &mut m.content {
                for block in blocks.iter_mut() {
                    block.cache_control = None;
                }
            }
        }
    }
}

/// Mark the last user-content block — skipped when it is a `thinking` block
/// (never valid here, but mirrored from Python's guard for safety).
fn mark_last_user_block(content: &mut StringOrBlocks<AnthropicUserContentBlock>) {
    let StringOrBlocks::Blocks(blocks) = content else {
        return;
    };
    if blocks.is_empty() {
        return;
    }
    if user_content_block_view(blocks.last().expect("non-empty")).type_str() == "thinking" {
        return;
    }

    // `last_block.cache_control = ...` — Python sets the attribute on the
    // object; here the discriminant is trusted to reach the right variant.
    let last = blocks.last_mut().expect("non-empty");
    match last {
        AnthropicUserContentBlock::Text(b) => b.cache_control = Some(ephemeral()),
        AnthropicUserContentBlock::Image(b) => b.cache_control = Some(ephemeral()),
        AnthropicUserContentBlock::Document(b) => b.cache_control = Some(ephemeral()),
        AnthropicUserContentBlock::ToolResult(b) => b.cache_control = Some(ephemeral()),
    }
}

/// Mark the last assistant-content block, skipping `thinking` blocks.
fn mark_last_assistant_block(content: &mut StringOrBlocks<AnthropicAssistantContentBlock>) {
    let StringOrBlocks::Blocks(blocks) = content else {
        return;
    };
    if blocks.is_empty() {
        return;
    }
    if assistant_content_block_view(blocks.last().expect("non-empty")).type_str() == "thinking" {
        return;
    }

    let last = blocks.last_mut().expect("non-empty");
    match last {
        AnthropicAssistantContentBlock::Text(b) => b.cache_control = Some(ephemeral()),
        AnthropicAssistantContentBlock::ToolUse(b) => b.cache_control = Some(ephemeral()),
        AnthropicAssistantContentBlock::Thinking(_) => {}
    }
}

/// Mark the last system-content block (always a text block).
fn mark_last_system_block(content: &mut StringOrBlocks<AnthropicTextBlock>) {
    let StringOrBlocks::Blocks(blocks) = content else {
        return;
    };
    if blocks.is_empty() {
        return;
    }
    blocks.last_mut().expect("non-empty").cache_control = Some(ephemeral());
}