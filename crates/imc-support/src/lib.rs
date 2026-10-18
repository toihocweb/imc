//! Support modules for Claude Code integration: context packing, search-limit
//! policy and the full-stack endpoint bridge used by the tracer.
//!
//! Ports of the corresponding Python modules:
//!   main/search/context.py   — estimate_tokens / pack_context
//!   main/search/limits.py    — resolve_search_limit
//!   main/core/endpoint_bridge.py — normalize_url_path + EndpointBridge

pub mod context;
pub mod endpoints;
pub mod limits;
