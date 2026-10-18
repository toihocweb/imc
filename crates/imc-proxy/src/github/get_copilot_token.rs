//! Port of `main/proxy/services/github/get_copilot_token.py`.
//!
//! Exchanges the locally stored GitHub `oauth_token` for a short-lived Copilot
//! token and persists it on the process-wide `state`.

use anyhow::{Context, Result};
use serde_json::Value;

use imc_proxy_types::api_config::{get_github_api_base_url, github_headers};
use imc_proxy_types::logger::create_handler_logger;
use imc_proxy_types::state::{state, with_state_mut};

use super::get_oauth_token::get_oauth_token;
use super::get_user::{auth_state, header_map, http_error};

/// Port of `get_copilot_token` — returns the raw JSON body
/// (`{"token": "..."}`) like the Python version.
pub async fn get_copilot_token() -> Result<Value> {
    let github_token = state()
        .github_token
        .clone()
        .filter(|t| !t.is_empty())
        .or_else(get_oauth_token)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Could not find GitHub Copilot oauth_token. Please sign in to Copilot first."
            )
        })?;

    let url = format!("{}/copilot_internal/v2/token", get_github_api_base_url());
    let headers = github_headers(&auth_state(Some(&github_token)));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("build http client")?;
    let response = client
        .get(&url)
        .headers(header_map(&headers))
        .send()
        .await
        .context("get copilot token")?;

    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        create_handler_logger("main")
            .error(&format!("Failed to get Copilot token response body: {text}"));
        return Err(http_error("Failed to get Copilot token", status, text));
    }

    serde_json::from_str(&text).context("parse copilot token json")
}

/// Port of the `state.copilotToken = token_data.get("token")` assignment that
/// follows every successful call.
pub async fn refresh_copilot_token() -> Result<String> {
    let token_data = get_copilot_token().await?;
    let token = token_data
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if token.is_empty() {
        anyhow::bail!("Copilot token response carried no token");
    }
    with_state_mut(|s| s.copilot_token = Some(token.clone()));
    Ok(token)
}