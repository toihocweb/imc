//! Port of `main/proxy/services/copilot/create_responses.py` — POST an OpenAI
//! Responses payload to Copilot's `/responses` endpoint.
//!
//! [`CopilotTokenProvider`] is defined here because the Python's
//! `get_copilot_token()` import lives in the sibling `imc-proxy` crate.
