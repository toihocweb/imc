//! Port of `main/proxy/services/github/**` — the GitHub/Copilot auth chain.
//!
//! ```text
//! get_oauth_token  ──▶  get_copilot_token  ──▶  Copilot upstream calls
//! get_device_code  ──▶  poll_access_token ──▶  state.github_token
//! ```

pub mod get_copilot_token;
pub mod get_copilot_usage;
pub mod get_device_code;
pub mod get_oauth_token;
pub mod get_user;
pub mod poll_access_token;

pub use get_copilot_token::{get_copilot_token, refresh_copilot_token};
pub use get_copilot_usage::{get_copilot_account_type, get_copilot_usage, CopilotAccountType};
pub use get_device_code::get_device_code;
pub use get_oauth_token::get_oauth_token;
pub use get_user::get_github_user;
pub use poll_access_token::poll_access_token;

use anyhow::Result;
use futures::future::BoxFuture;
use imc_proxy_copilot::chat_completions::CopilotTokenProvider;

/// The [`CopilotTokenProvider`] `imc-proxy-copilot` injects into its upstream
/// calls. One zero-sized type keeps the trait object-free and matches Python's
/// module-level `get_copilot_token` import.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitHubTokenProvider;

impl CopilotTokenProvider for GitHubTokenProvider {
    fn get_copilot_token(&self) -> BoxFuture<'_, Result<String>> {
        Box::pin(async move { refresh_copilot_token().await })
    }
}

/// Convenience: the provider instance every route passes to the copilot crate.
pub fn token_provider() -> GitHubTokenProvider {
    GitHubTokenProvider
}