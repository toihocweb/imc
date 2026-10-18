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
const CONTEXT_TOKEN_BUDGET: i64 = 6000;

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
}
