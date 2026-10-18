//! Port of `main/proxy/routes/system.py` — `/health` and `/v1/quota`.

use axum::body::Body;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use imc_proxy_types::api_config::{get_github_api_base_url, github_headers};
use imc_proxy_types::state::state;
use serde_json::{json, Value};

use crate::github::get_oauth_token;

/// Port of `health_check`.
pub async fn health_check() -> Response {
    json_response(&json!({"status": "ok"}))
}

/// Port of `get_quota` — Copilot quota snapshots from the GitHub API.
pub async fn get_quota() -> Response {
    let Some(gh_token) = get_oauth_token().or_else(|| state().github_token.clone()) else {
        return json_response(&json!({"error": "No GitHub token found"}));
    };

    let mut auth_state = state();
    auth_state.github_token = Some(gh_token);

    let url = format!("{}/copilot_internal/user", get_github_api_base_url());
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => return json_response(&json!({ "error": format!("Network error: {e}") })),
    };

    let send = client
        .get(&url)
        .header("authorization", format!("token {}", auth_state.github_token.clone().unwrap_or_default()))
        .send()
        .await;

    match send {
        Ok(resp) => {
            let status = resp.status().as_u16();
            if !(200..300).contains(&status) {
                return json_response(&json!({ "error": format!("HTTP {status}") }));
            }
            let data: Value = resp.json().await.unwrap_or(Value::Null);
            json_response(&json!({
                "quota_reset_date": data.get("quota_reset_date").cloned().unwrap_or(Value::Null),
                "quota_snapshots": data.get("quota_snapshots").cloned().unwrap_or(json!({})),
            }))
        }
        Err(e) => json_response(&json!({ "error": format!("Network error: {e}") })),
    }
}

/// Axum `Json` equivalent that always emits JSON.
pub fn json_response(value: &Value) -> Response {
    Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(Body::from(value.to_string()))
        .expect("json response")
}

/// Headers for the GitHub user API, exposed for the tests below.
pub fn quota_headers() -> Vec<(String, String)> {
    github_headers(&state()).into_iter().collect()
}

/// Routes served by this module.
pub fn router() -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/v1/quota", get(get_quota))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_ok() {
        let resp = health_check().await;
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.headers().get("content-type").unwrap(), "application/json");
    }

    #[test]
    fn quota_headers_carry_authorization() {
        let headers = quota_headers();
        assert!(headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("authorization")));
    }
}