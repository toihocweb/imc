//! Python's default `imc search` mode: retrieve, pack the context, ask the LLM.
//!
//! Port of the agentic branch of `main/cli/commands/search.py` (the one that
//! runs unless `-r/--raw` is passed) plus the `pack_context` step from
//! `main/search/context.py`. `imc_support::context::pack_context` was ported and
//! tested but never wired to a caller; this is its caller.
//!
//! Failure policy, chosen so an offline machine still gets usable output:
//! * no transport to the LLM endpoint -> caller falls back to raw hits and says
//!   so on stderr (`AskOutcome::Unavailable`);
//! * a reachable endpoint that answers with an HTTP error or unparseable body
//!   -> hard error, like the Python exception;
//! * `-r/--raw` and `--json` never call the LLM at all.

use std::time::Duration;

use anyhow::{bail, Context, Result};

use imc_core::config::Config;
use imc_core::types::SearchHit;
use imc_support::context::{pack_context, ContextResult};

/// `DEFAULT_CONTEXT_TOKEN_BUDGET` from `main/search/context.py:9`.
const CONTEXT_TOKEN_BUDGET: i64 = 3000;

/// The tuned retrieval prompt, kept in-crate so the binary carries it.
/// `docs/search_prompt_tuned.md` keeps the same text plus the tuning method and
/// the measured before/after table; edit both together.
const DEFAULT_PROMPT: &str = include_str!("prompts/imc_search.md");

/// What `ask` came back with.
pub enum AskOutcome {
    /// The model answered.
    Answer(String),
    /// The endpoint could not be reached; the caller should print raw hits.
    Unavailable(String),
}

/// The prompt template: an operator override under the data dir wins, mirroring
/// Python's stored `imc_search` prompt.
pub fn prompt_template(cfg: &Config) -> String {
    let override_path = cfg.data_dir.join("prompts").join("imc_search.md");
    std::fs::read_to_string(&override_path).unwrap_or_else(|_| DEFAULT_PROMPT.to_string())
}

/// Python's `prompt_template.format(context=..., query=...)`, with the same
/// fallback `e2e/prompt_eval.py` uses when the template has no placeholders.
pub fn build_prompt(template: &str, context: &str, query: &str) -> String {
    if template.contains("{context}") || template.contains("{query}") {
        template.replace("{context}", context).replace("{query}", query)
    } else {
        format!(
            "{template}\n\n<codebase_context>\n{context}\n</codebase_context>\n\nQuestion: {query}\n"
        )
    }
}

/// `pack_context([SearchResult(...) for hit in hits])`.
///
/// A hit's `text` already carries the `// graph:` evidence that Python appended
/// into `get_text()`, so it maps to `text` while `ast_context` stays empty.
pub fn context_from_hits(hits: &[SearchHit]) -> String {
    let results: Vec<ContextResult> = hits
        .iter()
        .map(|h| {
            ContextResult::new(&h.file, &h.text, h.start_line as i64, h.end_line as i64)
        })
        .collect();
    pack_context(&results, CONTEXT_TOKEN_BUDGET)
}

/// POST `{base_url}/chat/completions` with the packed context.
pub fn ask(cfg: &Config, query: &str, context: &str) -> Result<AskOutcome> {
    let base = cfg.llm_base_url();
    if base.trim().is_empty() {
        return Ok(AskOutcome::Unavailable(
            "no LLM endpoint configured (llm.base_url)".to_string(),
        ));
    }
    if cfg.llm_model_configured().is_none() {
        eprintln!(
            "[imc] warning: llm.model is not set — falling back to the built-in default '{}'. \
Set the model your endpoint actually serves: `imc config set llm.model <name>` \
(or `imc config setup`); endpoints usually answer 404 for the default.",
            cfg.llm_model()
        );
    }
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let prompt = build_prompt(&prompt_template(cfg), context, query);
    let body = serde_json::json!({
        "model": cfg.llm_model(),
        "messages": [{ "role": "user", "content": prompt }],
        "stream": false,
    });

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        // Read the status and the body ourselves: `Error::StatusCode` carries
        // only the number, and "which key/model is wrong" lives in the body.
        .http_status_as_error(false)
        .build()
        .into();
    let mut req = agent
        .post(&url)
        .header("Content-Type", "application/json");
    let key = cfg.llm_api_key();
    if !key.is_empty() {
        req = req.header("Authorization", format!("Bearer {key}"));
    }

    let mut resp = match req.send_json(&body) {
        Ok(r) => r,
        Err(e) => {
            return Ok(AskOutcome::Unavailable(format!(
                "LLM endpoint {url} unreachable ({e})"
            )))
        }
    };
    let status = resp.status();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    if !status.is_success() {
        let detail: String = text.chars().take(300).collect();
        bail!(
            "LLM {url} returned HTTP {}: {}{}",
            status.as_u16(),
            detail.trim(),
            if status.as_u16() == 404 || status.as_u16() == 400 {
                format!("\n  (model {:?} may not exist at this endpoint: set `imc config set llm.model <name>`)", cfg.llm_model())
            } else {
                String::new()
            }
        );
    }

    let value: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("LLM response from {url} was not JSON"))?;
    let answer = value
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string());
    match answer {
        Some(a) => Ok(AskOutcome::Answer(a)),
        None => bail!("LLM response from {url} had no choices[0].message.content"),
    }
}

/// What `rerank` came back with.
#[derive(Debug, Clone)]
pub enum RerankOutcome {
    /// LLM returned ranked/filtered hits
    Reranked(Vec<SearchHit>),
    /// LLM endpoint unavailable, caller should fall back to original hits
    Unavailable(String),
}

/// Prompt asking the LLM to rerank candidate code chunks.
pub fn build_rerank_prompt(query: &str, hits: &[SearchHit]) -> String {
    let mut candidates = String::new();
    for (i, h) in hits.iter().enumerate() {
        let sym = h.sym_name.as_deref().unwrap_or("none");
        let preview = if !h.snippet.trim().is_empty() {
            &h.snippet
        } else {
            &h.text
        };
        let preview_lines = preview.lines().take(10).collect::<Vec<_>>().join("\n");
        candidates.push_str(&format!(
            "[{}] {}:{}-{} (symbol: {})\n```\n{}\n```\n\n",
            i + 1,
            h.file,
            h.start_line,
            h.end_line,
            sym,
            preview_lines
        ));
    }

    format!(
r#"You are a code search reranker.
Given a query and numbered candidate code chunks from a search index, select and rank the candidates that are genuinely relevant to the query.
Exclude boilerplate, mock fixtures, test assertions, and false-positive keyword matches.

Query: "{query}"

Candidates:
{candidates}
Respond ONLY with a JSON object in this format:
{{"selected": [1, 5, 2]}}
where the numbers are 1-based candidate indices in order of relevance (best first).
If none of the candidates are relevant, return {{"selected": []}}."#
    )
}

/// Parse the LLM's selection JSON: `{"selected": [1, 5, 2]}` or `[1, 5, 2]`.
pub fn parse_rerank_selection(text: &str, total_candidates: usize) -> Option<Vec<usize>> {
    let trimmed = text.trim();
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(arr) = val.get("selected").and_then(|v| v.as_array()) {
            return Some(extract_indices(arr, total_candidates));
        }
        if let Some(arr) = val.as_array() {
            return Some(extract_indices(arr, total_candidates));
        }
    }

    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if end > start {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&trimmed[start..=end]) {
                    if let Some(arr) = val.get("selected").and_then(|v| v.as_array()) {
                        return Some(extract_indices(arr, total_candidates));
                    }
                }
            }
        }
    }

    if let Some(start) = trimmed.find('[') {
        if let Some(end) = trimmed.rfind(']') {
            if end > start {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&trimmed[start..=end]) {
                    if let Some(arr) = val.as_array() {
                        return Some(extract_indices(arr, total_candidates));
                    }
                }
            }
        }
    }

    None
}

fn extract_indices(arr: &[serde_json::Value], total: usize) -> Vec<usize> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for v in arr {
        if let Some(n) = v.as_u64() {
            let idx = n as usize;
            if idx >= 1 && idx <= total && seen.insert(idx) {
                result.push(idx - 1);
            }
        }
    }
    result
}

/// Rerank candidate hits using the configured LLM.
pub fn rerank(cfg: &Config, query: &str, hits: &[SearchHit]) -> Result<RerankOutcome> {
    if hits.is_empty() {
        return Ok(RerankOutcome::Reranked(Vec::new()));
    }
    let base = cfg.llm_base_url();
    if base.trim().is_empty() {
        return Ok(RerankOutcome::Unavailable(
            "no LLM endpoint configured (llm.base_url)".to_string(),
        ));
    }
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let prompt = build_rerank_prompt(query, hits);
    let body = serde_json::json!({
        "model": cfg.llm_model(),
        "messages": [{ "role": "user", "content": prompt }],
        "stream": false,
    });

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut req = agent
        .post(&url)
        .header("Content-Type", "application/json");
    let key = cfg.llm_api_key();
    if !key.is_empty() {
        req = req.header("Authorization", format!("Bearer {key}"));
    }

    let mut resp = match req.send_json(&body) {
        Ok(r) => r,
        Err(e) => {
            return Ok(RerankOutcome::Unavailable(format!(
                "LLM endpoint {url} unreachable ({e})"
            )));
        }
    };
    let status = resp.status();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    if !status.is_success() {
        let detail: String = text.chars().take(200).collect();
        return Ok(RerankOutcome::Unavailable(format!(
            "LLM returned HTTP {}: {}",
            status.as_u16(),
            detail.trim()
        )));
    }

    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return Ok(RerankOutcome::Unavailable("LLM response was not valid JSON".to_string())),
    };
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str());

    match content {
        Some(raw) => {
            if let Some(indices) = parse_rerank_selection(raw, hits.len()) {
                let mut reranked = Vec::with_capacity(indices.len());
                for idx in indices {
                    reranked.push(hits[idx].clone());
                }
                Ok(RerankOutcome::Reranked(reranked))
            } else {
                Ok(RerankOutcome::Reranked(hits.to_vec()))
            }
        }
        None => Ok(RerankOutcome::Unavailable("No content in LLM response".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_placeholders_are_filled() {
        let got = build_prompt("ctx={context}\nq={query}", "CTX", "Q");
        assert_eq!(got, "ctx=CTX\nq=Q");
    }

    #[test]
    fn placeholderless_prompt_gets_the_fallback_block() {
        let got = build_prompt("only rules", "CTX", "Q");
        assert!(got.contains("<codebase_context>\nCTX\n</codebase_context>"), "{got}");
        assert!(got.ends_with("Question: Q\n"), "{got}");
    }

    #[test]
    fn embedded_prompt_is_the_tuned_one_and_teaches_real_flags() {
        let t = DEFAULT_PROMPT;
        assert!(t.contains("<codebase_context>"), "tuned prompt body missing");
        assert!(t.contains("{context}") && t.contains("{query}"));
        // `-d` is depth (Python parity); reverse tracing is `--callers`.
        assert!(!t.contains("-d callers"), "prompt teaches the old -d flag: {t}");
        assert!(t.contains("--callers"));
    }

    #[test]
    fn hits_pack_into_context_with_ranges() {
        let hits = vec![SearchHit {
            score: 1.0,
            file: "src/a.py".into(),
            start_line: 3,
            end_line: 9,
            text: "def a():\n    return 1\n".into(),
            snippet: "def a()".into(),
            sym_name: Some("a".into()),
            sym_kind: Some("function".into()),
            source: "hybrid",
        }];
        let ctx = context_from_hits(&hits);
        assert!(ctx.contains("src/a.py"), "{ctx}");
        assert!(ctx.contains("return 1"), "{ctx}");
    }

    #[test]
    fn empty_hits_pack_to_nothing() {
        assert!(context_from_hits(&[]).is_empty());
    }

    #[test]
    fn parses_rerank_selection_formats() {
        // Standard object
        let parsed = parse_rerank_selection(r#"{"selected": [2, 1, 4]}"#, 5).unwrap();
        assert_eq!(parsed, vec![1, 0, 3]);

        // Out of bound indices filtered out
        let filtered = parse_rerank_selection(r#"{"selected": [0, 2, 99]}"#, 3).unwrap();
        assert_eq!(filtered, vec![1]);

        // Markdown code fence wrapped
        let fenced = parse_rerank_selection("```json\n{\"selected\": [3, 1]}\n```", 3).unwrap();
        assert_eq!(fenced, vec![2, 0]);

        // Bare array
        let arr = parse_rerank_selection("[1, 2]", 2).unwrap();
        assert_eq!(arr, vec![0, 1]);
    }
}
