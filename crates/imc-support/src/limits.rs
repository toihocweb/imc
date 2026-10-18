//! Search result limit policy without retrieval dependencies.
//! Faithful port of `main/search/limits.py`.

use regex::Regex;
use std::sync::OnceLock;

fn limit_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?:limit|top|k|chunks?)[:\s=]+(\d+)|(\d+)\s*(?:chunks?|results?|nodes?)")
            .expect("valid limit regex")
    })
}

/// Dynamically determine the number of chunks to retrieve:
/// 1. Explicit limit passed by caller / CLI flag (1 to 20).
/// 2. Explicit chunk directive requested in query or directive
///    (e.g. '8 chunks', 'limit: 7', 'top 6').
/// 3. Heuristic based on query intent & scope:
///    - Broad / architecture / end-to-end flow -> 8 chunks
///    - Specific symbol / definition / lookup -> 3-4 chunks
///    - Default base -> 5 chunks
pub fn resolve_search_limit(query: &str, directive: &str, explicit_limit: Option<i64>) -> i64 {
    if let Some(n) = explicit_limit {
        if n > 0 {
            return n.max(1).min(20);
        }
    }

    let combined = format!("{query} {directive}").to_lowercase();
    if let Some(caps) = limit_re().captures(&combined) {
        let val = caps
            .get(1)
            .or_else(|| caps.get(2))
            .and_then(|m| m.as_str().parse::<i64>().ok());
        if let Some(v) = val {
            if (1..=25).contains(&v) {
                return v;
            }
        }
    }

    const BROAD: &[&str] = &[
        "architecture",
        "overview",
        "workflow",
        "data flow",
        "pipeline",
        "end to end",
        "end-to-end",
        "how does the whole",
        "all components",
        "relationships",
        "structure of",
        "lifecycle",
        "compare",
        "system design",
    ];
    if BROAD.iter().any(|k| combined.contains(k)) {
        return 8;
    }

    const FOCUSED: &[&str] = &[
        "where is",
        "find function",
        "definition of",
        "what line",
        "signature of",
        "defined in",
        "which file defines",
    ];
    if FOCUSED.iter().any(|k| combined.contains(k)) {
        return 3;
    }

    5
}

#[cfg(test)]
mod tests {
    use super::resolve_search_limit;

    #[test]
    fn explicit_limit_clamped_to_1_20() {
        assert_eq!(resolve_search_limit("anything", "", Some(7)), 7);
        assert_eq!(resolve_search_limit("anything", "", Some(0)), 5);
        assert_eq!(resolve_search_limit("anything", "", Some(-3)), 5);
        assert_eq!(resolve_search_limit("anything", "", Some(50)), 20);
        assert_eq!(resolve_search_limit("anything", "", Some(1)), 1);
    }

    #[test]
    fn numeric_directive_variants() {
        // 'limit: 7'
        assert_eq!(resolve_search_limit("limit: 7", "", None), 7);
        // 'top 6'
        assert_eq!(resolve_search_limit("top 6 widgets", "", None), 6);
        // '8 chunks'
        assert_eq!(resolve_search_limit("give me 8 chunks", "", None), 8);
        // 'k=4'
        assert_eq!(resolve_search_limit("k=4", "", None), 4);
        // '12 results'
        assert_eq!(resolve_search_limit("return 12 results", "", None), 12);
        // directive-only source
        assert_eq!(resolve_search_limit("plain query", "limit=9", None), 9);
    }

    #[test]
    fn out_of_range_numeric_falls_through() {
        // 26 is outside 1..=25, so falls to heuristic (default 5)
        assert_eq!(resolve_search_limit("26 chunks", "", None), 5);
        // 0 is outside range too
        assert_eq!(resolve_search_limit("top 0", "", None), 5);
    }

    #[test]
    fn broad_keywords_return_8() {
        assert_eq!(resolve_search_limit("explain the architecture", "", None), 8);
        assert_eq!(resolve_search_limit("end-to-end data flow", "", None), 8);
        assert_eq!(resolve_search_limit("system design overview", "", None), 8);
    }

    #[test]
    fn focused_keywords_return_3() {
        assert_eq!(resolve_search_limit("where is parse_config", "", None), 3);
        assert_eq!(resolve_search_limit("definition of resolve", "", None), 3);
    }

    #[test]
    fn default_is_5() {
        assert_eq!(resolve_search_limit("parse the config", "", None), 5);
        assert_eq!(resolve_search_limit("", "", None), 5);
    }
}
