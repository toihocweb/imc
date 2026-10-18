//! Port of `main/proxy/services/copilot/**` — Anthropic Messages ⇄ OpenAI
//! Responses ⇄ Chat Completions translation (stream + non-stream).

pub mod chat_completions;
pub mod context_filters;
pub mod create_messages;
pub mod create_responses;
pub mod non_stream_translation;
pub mod preprocess;
pub mod python_json;
pub mod responses_stream_translation;
pub mod responses_translation;
pub mod stream_translation;
