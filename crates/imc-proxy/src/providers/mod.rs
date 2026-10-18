//! Port of `main/proxy/services/providers/**` — upstream routing + forwarding.

pub mod provider_proxy;

pub use provider_proxy::{
    build_provider_upstream_headers, decide_route, forward_provider_alpha_search,
    forward_provider_chat_completions, forward_provider_images, forward_provider_messages,
    forward_provider_models, forward_provider_responses, resolve_provider,
    resolve_provider_request_url, route_for_model, strip_response_headers, RouteDecision,
    RouteTarget,
};