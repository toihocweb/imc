//! Port of `main/proxy/services/copilot/create_responses.py` — POST an OpenAI
//! Responses payload to Copilot's `/responses` endpoint.
//!
//! [`CopilotTokenProvider`] is injected because the Python's
//! `get_copilot_token()` import lives in the sibling `imc-proxy` crate, which
//! depends on this one.

use anyhow::{Context, Result};
use imc_proxy_types::api_config::{copilot_base_url, github_copilot_headers};
use imc_proxy_types::state::state;
use imc_proxy_types::types::responses::ResponsesPayload;

use crate::chat_completions::{ensure_copilot_token, header_map, CopilotTokenProvider};

/// Port of `create_responses` — build the request headers and POST the
/// payload. Python sends this one non-streaming.
pub async fn create_responses<P: CopilotTokenProvider + ?Sized>(
    provider: &P,
    payload: &ResponsesPayload,
) -> Result<reqwest::Response> {
    // 1. Ensure we have a valid token.
    ensure_copilot_token(provider).await?;

    let st = state();
    let url = format!("{}/responses", copilot_base_url(&st));

    let mut headers = github_copilot_headers(&st, None, false);
    // Python: default to user, could be agent.
    headers.insert(
        "x-initiator".to_string(),
        crate::chat_completions::INITIATOR_USER.to_string(),
    );

    let body = serde_json::to_value(payload).context("serialize responses payload")?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .context("build http client")?;

    // 2. Send request to Copilot.
    let request = client
        .post(&url)
        .headers(header_map(&headers))
        .json(&body)
        .build()
        .context("build responses request")?;

    let response = client.execute(request).await.context("post responses")?;
    Ok(response)
}