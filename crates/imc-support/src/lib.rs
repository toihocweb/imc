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

/// Lower-case the first character without assuming it is one byte
/// (`&s[..1]` panics when the first char is multi-byte).
pub fn lower_first(s: &str) -> String {
    let mut it = s.chars();
    match it.next() {
        Some(c) => c.to_lowercase().collect::<String>() + it.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::lower_first;

    #[test]
    fn lower_first_is_char_safe() {
        assert_eq!(lower_first("GetMapping"), "getMapping");
        assert_eq!(lower_first(""), "");
        assert_eq!(lower_first("École"), "école");
        assert_eq!(lower_first("갱"), "갱");
    }
}
