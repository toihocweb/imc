//! Port of `main/proxy/services/copilot/create_messages.py` — POST an
//! Anthropic Messages payload to Copilot's `/v1/messages` endpoint.
//!
//! ## Port notes
//!
//! * Python's `get_copilot_token()` lives in the sibling `imc-proxy` crate, so
//!   the token fetch is injected via [`CopilotTokenProvider`].
//! * Python attaches the owning `httpx.AsyncClient` to the response
//!   (`response._imc_client`) so the caller can close both; a `reqwest`
//!   response owns its pooled connection, so [`aclose_stream_response`] just
//!   drops it.
//! * Python's `client.send(request, stream=True)` returns as soon as headers
//!   arrive. A `reqwest::Response` body is already lazy — reading it streams —
//!   so the same "forward SSE chunks the instant they're produced" behaviour
//!   holds without an extra flag.

use anyhow::{Context, Result};
use imc_proxy_types::api_config::{copilot_base_url, github_copilot_headers};
use imc_proxy_types::state::state;
use imc_proxy_types::types::anthropic::{
    AnthropicInputMessage, AnthropicMessagesPayload, StringOrBlocks,
};

use crate::chat_completions::{
    ensure_copilot_token, header_map, CopilotTokenProvider, INITIATOR_USER,
};
use crate::non_stream_translation::{
    assistant_content_block_view, tool_result_content_block_view, user_content_block_view,
    ContentBlock,
};

/// Port of `aclose_stream_response`.
///
/// Python closes the streaming response and the `httpx.AsyncClient` it
/// attached. `reqwest`'s client is pooled and shared, so dropping the
/// response releases its connection; there is no second handle to close.
pub async fn aclose_stream_response(response: reqwest::Response) {
    // Dropping the response closes the body stream and returns the connection
    // to the pool (or drops it if the body was not fully drained).
    drop(response);
}

/// Port of the vision check that precedes the request: Copilot rejects an
/// image payload unless the `copilot-vision-request` header is set.
pub fn detect_vision(payload: &AnthropicMessagesPayload) -> bool {
    for message in &payload.messages {
        match message {
            AnthropicInputMessage::User(m) => {
                let StringOrBlocks::Blocks(blocks) = &m.content else {
                    continue;
                };
                for block in blocks.iter().map(user_content_block_view) {
                    if block_has_image(&block) {
                        return true;
                    }
                }
            }
            AnthropicInputMessage::Assistant(m) => {
                let StringOrBlocks::Blocks(blocks) = &m.content else {
                    continue;
                };
                for block in blocks.iter().map(assistant_content_block_view) {
                    if block_has_image(&block) {
                        return true;
                    }
                }
            }
            // A system message is a text-block list in the Anthropic schema.
            AnthropicInputMessage::System(_) => {}
        }
    }
    false
}

/// True for an image block, or a `tool_result` whose block-list content
/// carries an image.
fn block_has_image(block: &ContentBlock) -> bool {
    match block {
        ContentBlock::Image { .. } => true,
        ContentBlock::ToolResult { content, .. } => match content {
            StringOrBlocks::Blocks(inner) => inner
                .iter()
                .any(|b| tool_result_content_block_view(b).type_str() == "image"),
            StringOrBlocks::Text(_) => false,
        },
        _ => false,
    }
}

/// Port of the `is_initiate_request` computation: true when the last message
/// is a user message carrying anything other than tool results.
///
/// The header tells Copilot whether a human started the turn (`user`) or an
/// agent did (`agent`), which gates the abuse-detection path upstream.
pub fn is_initiate_request(payload: &AnthropicMessagesPayload) -> bool {
    let Some(last_message) = payload.messages.last() else {
        return false;
    };
    let role = match last_message {
        imc_proxy_types::types::anthropic::AnthropicInputMessage::User(m) => &m.role,
        imc_proxy_types::types::anthropic::AnthropicInputMessage::Assistant(m) => &m.role,
        imc_proxy_types::types::anthropic::AnthropicInputMessage::System(m) => &m.role,
    };
    if role != "user" {
        return false;
    }

    // The role check above guarantees this is a user message.
    let AnthropicInputMessage::User(m) = last_message else {
        return false;
    };

    // `isinstance(content, list)` -> any block that is not a tool result;
    // otherwise a bare string (including "") is an initiated turn.
    match &m.content {
        StringOrBlocks::Blocks(blocks) => blocks
            .iter()
            .any(|b| user_content_block_view(b).type_str() != "tool_result"),
        StringOrBlocks::Text(_) => true,
    }
}

/// Port of `create_messages` — build the request headers and POST the payload.
///
/// The response is returned unread, so the route can forward SSE chunks the
/// instant they're produced upstream.
pub async fn create_messages<P: CopilotTokenProvider + ?Sized>(
    provider: &P,
    payload: &AnthropicMessagesPayload,
    anthropic_beta: Option<&str>,
) -> Result<reqwest::Response> {
    ensure_copilot_token(provider).await?;

    let st = state();
    let url = format!("{}/v1/messages", copilot_base_url(&st));

    let enable_vision = detect_vision(payload);
    let initiate = is_initiate_request(payload);
    let request_id = imc_proxy_types::utils::random_uuid_v4();

    let mut headers = github_copilot_headers(&st, Some(&request_id), enable_vision);
    headers.insert(
        "x-initiator".to_string(),
        if initiate {
            INITIATOR_USER.to_string()
        } else {
            "agent".to_string()
        },
    );
    if let Some(beta) = anthropic_beta.filter(|b| !b.is_empty()) {
        headers.insert("anthropic-beta".to_string(), beta.to_string());
    }

    // Python logs `<-- model: {payload.model}` at info level.

    let body = serde_json::to_value(payload).context("serialize messages payload")?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .context("build http client")?;

    let request = client
        .post(&url)
        .headers(header_map(&headers))
        .json(&body)
        .build()
        .context("build messages request")?;

    // Python: `await client.send(request, stream=True)`.
    let response = client.execute(request).await.context("post messages")?;
    Ok(response)
}