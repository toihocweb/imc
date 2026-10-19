//! Port of `main/proxy/**` — transparent LLM proxy on 127.0.0.1:12345 that
//! intercepts Anthropic/OpenAI traffic from Claude Code, injects imc search
//! context (system_reminder) and filters noisy tool results.

pub mod compression;
pub mod github;
pub mod hyperspace;
pub mod imc_memory;
pub mod providers;
pub mod routes;

pub const DEFAULT_PORT: u16 = 12345;

/// Run the proxy (blocking). `port` 0 = DEFAULT_PORT.
pub fn run(port: u16) -> anyhow::Result<()> {
    let port = if port == 0 { DEFAULT_PORT } else { port };
    routes::serve(port)
}
