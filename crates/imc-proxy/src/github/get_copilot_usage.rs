//! Port of `main/proxy/services/github/get_copilot_usage.py`.

use anyhow::Result;
use serde_json::Value;

use imc_proxy_types::api_config::{get_github_api_base_url, github_headers};
use imc_proxy_types::logger::create_handler_logger;
use imc_proxy_types::state::state;

use super::get_user::{auth_state, header_map, http_error};

/// The three Copilot plans this proxy distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopilotAccountType {
    Individual,
    Business,
    Enterprise,
}

impl CopilotAccountType {
    pub fn as_str(self) -> &'static str {
        match self {
            CopilotAccountType::Individual => "individual",
            CopilotAccountType::Business => "business",
            CopilotAccountType::Enterprise => "enterprise",
        }
    }

    /// Port of the plan-string sniffing in `get_copilot_account_type`.
    pub fn from_plan(plan: &str) -> Self {
        let plan = plan.to_lowercase();
        if plan.contains("enterprise") {
            CopilotAccountType::Enterprise
        } else if plan.contains("business") {
            CopilotAccountType::Business
        } else {
            CopilotAccountType::Individual
        }
    }
}

/// Port of `get_copilot_usage` — `None` when no GitHub token is available.
pub async fn get_copilot_usage(github_token: Option<&str>) -> Result<Option<Value>> {
    let st = state();
    let resolved = github_token
        .filter(|t| !t.is_empty())
        .or_else(|| st.github_token.as_deref().filter(|t| !t.is_empty()))
        .map(str::to_string);
    let Some(resolved) = resolved else {
        return Ok(None);
    };

    let url = format!(
        "{}/copilot_internal/user",
        get_github_api_base_url()
    );
    let headers = github_headers(&auth_state(Some(&resolved)));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let response = client.get(&url).headers(header_map(&headers)).send().await?;

    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        create_handler_logger("main")
            .error(&format!("Failed to get Copilot user response body: {text}"));
        return Err(http_error("Failed to get Copilot usage", status, text));
    }
    Ok(Some(serde_json::from_str(&text).unwrap_or(Value::Null)))
}

/// Port of `get_copilot_account_type`.
pub async fn get_copilot_account_type(github_token: Option<&str>) -> Result<CopilotAccountType> {
    let usage = get_copilot_usage(github_token)
        .await?
        .ok_or_else(|| anyhow::anyhow!("GitHub token not found"))?;
    let plan = usage
        .get("copilot_plan")
        .and_then(Value::as_str)
        .unwrap_or("");
    Ok(CopilotAccountType::from_plan(plan))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_sniffing_matches_python_order() {
        assert_eq!(
            CopilotAccountType::from_plan("Copilot Enterprise"),
            CopilotAccountType::Enterprise
        );
        assert_eq!(
            CopilotAccountType::from_plan("Copilot Business"),
            CopilotAccountType::Business
        );
        assert_eq!(
            CopilotAccountType::from_plan("Copilot Pro"),
            CopilotAccountType::Individual
        );
        assert_eq!(
            CopilotAccountType::from_plan(""),
            CopilotAccountType::Individual
        );
        assert_eq!(CopilotAccountType::Business.as_str(), "business");
    }

    #[tokio::test]
    async fn usage_without_token_is_none() {
        assert!(get_copilot_usage(Some("")).await.unwrap().is_none());
    }
}