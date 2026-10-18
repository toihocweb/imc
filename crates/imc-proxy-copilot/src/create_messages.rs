//! Port of `main/proxy/services/copilot/create_messages.py` — POST an
//! Anthropic Messages payload to Copilot's `/v1/messages` endpoint.
//!
//! [`CopilotTokenProvider`] is defined here because the Python's
//! `get_copilot_token()` import lives in the sibling `imc-proxy` crate.
