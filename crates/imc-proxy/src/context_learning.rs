//! Learning a filter rule for a tool result, with a cheap model.
//!
//! The context filter already knows how to apply a rule: a rule is
//! `tool` + `command_pattern` + a list of transforms, and
//! `execute_transforms_pipeline` runs it. The rules shipped with imc cover
//! three commands (Cypress, Maven, GitHub Actions). Everything else — the
//! long `npm run build`, the `docker build` with 400 progress lines, the
//! `terraform plan` with 2 000 unchanged resources — is filtered by the
//! generic path, which only folds duplicates.
//!
//! This module closes that gap: when a command produces a large result that
//! no rule matched, the command and a sample of its output are kept, and a
//! cheap model is asked to write the rule that keeps the signal and drops the
//! noise. The rule is checked against the sample before it is stored — it
//! has to be smaller, it has to keep the error lines, and its regexes have to
//! compile — and from then on the same command is filtered by it.
//!
//! Three properties keep this safe:
//!
//! * the model can only pick from the transform types the pipeline already
//!   implements, so a rule can never run anything but regexes and line
//!   selection,
//! * the `regex` crate matches in linear time, so a model-written pattern
//!   cannot hang the proxy,
//! * a rule that removes the failure lines of its own sample is rejected.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;

/// A tool result worth learning from.
#[derive(Debug, Clone)]
pub struct Observation {
    /// The tool that produced it, e.g. `Bash`.
    pub tool: String,
    /// The command that was run.
    pub command: String,
    /// How many characters the result had before filtering.
    pub chars_before: usize,
    /// A sample of the result (the head), used to write and check the rule.
    pub sample: String,
}

/// Below this a result is not worth a model call.
pub const MIN_LEARN_CHARS: usize = 4_000;

/// How much of the result is shown to the model.
const SAMPLE_CHARS: usize = 8_000;

/// Where the learned rules are kept.
///
/// A separate file from the user's own `tool_result_filters.json`: the user
/// may edit theirs at any time, and a learned rule must not be lost when
/// they do.
pub fn learned_filters_path() -> PathBuf {
    imc_proxy_copilot::context_filters::learned_filters_path()
}

/// Is learning enabled? Off in CI, and off with an explicit env var.
pub fn enabled() -> bool {
    if std::env::var("IMC_NO_LEARNED_FILTERS").is_ok() {
        return false;
    }
    if std::env::var("CI").is_ok() {
        return false;
    }
    true
}

/// The prompt the model is given.
///
/// The rule format is the one the pipeline already understands, and the
/// model is told to answer with JSON only — the answer is parsed, not
/// interpreted.
fn prompt(obs: &Observation) -> String {
    format!(
        r#"You write filter rules for a coding agent's tool output.

The output below is the result of one command. Write ONE JSON rule that keeps
everything that matters and drops the noise.

What must survive: test results (passed and failed), errors, warnings that
matter, file paths, counts, summaries, the last lines.
What can go: progress lines, repeated banners, timestamps, "downloading" noise,
unchanged listings, stack traces of passing tests.

Use only these transforms:
  {{"type": "remove_lines", "pattern": "<regex>"}}
  {{"type": "keep_lines", "pattern": "<regex>"}}
  {{"type": "regex_replace", "pattern": "<regex>", "replacement": "<text>", "flags": "m"}}
  {{"type": "truncate_lines", "direction": "head" | "tail" | "head_and_tail", "max_lines": 20}}
  {{"type": "strip_noise"}}

The rule shape:
{{
  "tool": "<the tool name>",
  "command_pattern": "<the stable prefix of the command: the program and its subcommand>",
  "match_type": "startswith",
  "transforms": [ ... ]
}}

Command: {cmd}
Tool: {tool}

Output:
{sample}

Answer with the rule JSON only. No prose, no markdown fence."#,
        cmd = obs.command,
        tool = obs.tool,
        sample = &obs.sample,
    )
}

/// Pull the rule out of the model's answer.
///
/// The answer is JSON, but models wrap JSON in prose and fences anyway, so
/// the first balanced object is taken.
pub fn parse_rule(answer: &str) -> Result<Value> {
    let trimmed = answer.trim();
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return Ok(v);
    }
    if let Some(start) = trimmed.find('{') {
        let bytes = trimmed.as_bytes();
        let mut depth = 0usize;
        let mut in_string = false;
        let mut escaped = false;
        for (i, b) in bytes.iter().enumerate().skip(start) {
            let c = *b as char;
            if in_string {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_string = false;
                }
                continue;
            }
            match c {
                '"' => in_string = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        let candidate = &trimmed[start..=i];
                        return serde_json::from_str::<Value>(candidate)
                            .map_err(|e| anyhow!("the rule is not valid JSON: {e}"));
                    }
                }
                _ => {}
            }
        }
    }
    bail!("the answer has no JSON object")
}

/// The transform types the pipeline implements.
const KNOWN_TRANSFORMS: &[&str] = &[
    "remove_lines",
    "keep_lines",
    "regex_replace",
    "truncate_lines",
    "head_and_tail",
    "head_tail",
    "strip_noise",
    "remove_section",
    "keep_sections",
];

/// Check a rule against the sample it was written for.
///
/// A rule is only worth storing if it shrinks the output, keeps the failure
/// lines and uses nothing the pipeline does not implement.
pub fn validate_rule(rule: &Value, sample: &str) -> std::result::Result<Value, String> {
    let transforms = rule
        .get("transforms")
        .and_then(Value::as_array)
        .ok_or_else(|| "the rule has no transforms".to_string())?;
    if transforms.is_empty() {
        return Err("the rule has no transforms".to_string());
    }
    for t in transforms {
        let ty = t
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| "a transform has no type".to_string())?;
        if !KNOWN_TRANSFORMS.contains(&ty) {
            return Err(format!("unknown transform type `{ty}`"));
        }
        if let Some(p) = t.get("pattern").and_then(Value::as_str) {
            regex::Regex::new(p).map_err(|e| format!("the pattern does not compile: {e}"))?;
        }
    }
    // The command pattern has to be a prefix of the command it was learned
    // from, or the rule would fire on a different command.
    let pattern = rule
        .get("command_pattern")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let command = rule
        .get("_command")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if !command.is_empty() && !pattern.is_empty() && !command.starts_with(&pattern) {
        return Err(format!(
            "the command pattern `{pattern}` is not a prefix of the command it was learned from"
        ));
    }

    // Apply it and look at the result.
    let filtered = imc_proxy_copilot::context_filters::execute_transforms_pipeline(
        sample,
        Some(transforms),
    );
    if filtered.trim().is_empty() {
        return Err("the rule empties the output".to_string());
    }
    let before = sample.chars().count();
    let after = filtered.chars().count();
    if after >= before {
        return Err(format!("the rule does not shrink the output ({before} → {after})"));
    }
    if after * 10 > before * 9 {
        return Err(format!(
            "the rule saves too little ({before} → {after}, under 10%)"
        ));
    }

    // The failure lines have to survive: the failure is the answer.
    for line in sample.lines() {
        let l = line.to_lowercase();
        let is_failure = l.contains("error")
            || l.contains("fail")
            || l.contains("panic")
            || l.contains("exception")
            || l.contains("fatal");
        if !is_failure || line.trim().is_empty() {
            continue;
        }
        if !filtered.lines().any(|f| f.trim() == line.trim()) {
            return Err(format!(
                "the rule drops a failure line: {}",
                line.trim().chars().take(80).collect::<String>()
            ));
        }
    }

    Ok(rule.clone())
}

/// Merge a validated rule into the learned-rules file.
pub fn store_rule(rule: &Value) -> Result<()> {
    let path = learned_filters_path();
    let mut rules: Vec<Value> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();

    // One rule per (tool, command pattern): a new one replaces the old.
    let tool = rule.get("tool").and_then(Value::as_str).unwrap_or("");
    let pattern = rule
        .get("command_pattern")
        .and_then(Value::as_str)
        .unwrap_or("");
    rules.retain(|r| {
        !(r.get("tool").and_then(Value::as_str) == Some(tool)
            && r.get("command_pattern").and_then(Value::as_str) == Some(pattern))
    });

    let mut stored = rule.clone();
    if let Some(obj) = stored.as_object_mut() {
        obj.insert("enabled".to_string(), json!(true));
        obj.insert(
            "id".to_string(),
            json!(format!(
                "learned_{}",
                &format!("{tool}_{pattern}")[..tool.len().min(24) + pattern.len().min(24)]
                    .replace(['/', ' ', ':'], "_")
            )),
        );
        obj.insert("source".to_string(), json!("learned"));
    }
    rules.push(stored);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let body = serde_json::to_string_pretty(&Value::Array(rules))?;
    std::fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}


/// The number of filter-learning tasks running right now.
fn running() -> &'static std::sync::atomic::AtomicUsize {
    static RUNNING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    &RUNNING
}

/// How many filter-learning tasks may run at once.
const MAX_CONCURRENT: usize = 2;

/// Learn a filter in the background, with a cap on the number of tasks.
///
/// The request must not wait for the model, and a burst of tool results must
/// not start a burst of model calls.
pub fn learn_in_background(observation: Observation) {
    use std::sync::atomic::Ordering;
    if running().load(Ordering::SeqCst) >= MAX_CONCURRENT {
        return;
    }
    running().fetch_add(1, Ordering::SeqCst);
    tokio::spawn(async move {
        let settings = imc_core::settings::load();
        let result = learn(observation, &settings).await;
        running().fetch_sub(1, Ordering::SeqCst);
        match result {
            Ok(rule) => eprintln!(
                "[imc] learned a filter for `{}` ({} transforms)",
                rule.get("command_pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?"),
                rule.get("transforms")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0)
            ),
            Err(e) => eprintln!("[imc] could not learn a filter: {e}"),
        }
    });
}

/// Ask the model for a rule and store it if it checks out.
pub async fn learn(obs: Observation, settings: &imc_core::settings::Settings) -> Result<Value> {
    let llm = settings
        .llm
        .as_ref()
        .ok_or_else(|| anyhow!("no llm settings"))?;
    let base = llm
        .base_url
        .as_ref()
        .ok_or_else(|| anyhow!("no llm.base_url"))?;
    let key = llm
        .api_key
        .as_ref()
        .ok_or_else(|| anyhow!("no llm.api_key"))?;
    let model = llm.model.clone().unwrap_or_else(|| "claude-haiku-4.5".to_string());

    let body = json!({
        "model": model,
        "messages": [{ "role": "user", "content": prompt(&obs) }],
        "stream": false,
        "temperature": 0.0,
    });
    // A timeout: the learning runs in the background, and a hung endpoint
    // would leave the task alive for ever.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| anyhow!("could not build the http client: {e}"))?;
    let resp = client
        .post(format!("{}/chat/completions", base.trim_end_matches('/')))
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow!("the model call failed: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("the model answered {status}: {}", text.chars().take(200).collect::<String>());
    }
    let parsed: Value = serde_json::from_str(&text)
        .map_err(|e| anyhow!("the model answer is not JSON: {e}"))?;
    let answer = parsed
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("the model answer has no content"))?;

    let mut rule = parse_rule(answer)?;
    // The command the rule was learned from: the validator checks that the
    // pattern is a prefix of it.
    if let Some(obj) = rule.as_object_mut() {
        obj.insert("_command".to_string(), json!(obs.command));
    }
    let rule = validate_rule(&rule, &obs.sample)
        .map_err(|e| anyhow!("the rule was rejected: {e}"))?;

    // The internal field is not part of the stored rule.
    let mut stored = rule.clone();
    if let Some(obj) = stored.as_object_mut() {
        obj.remove("_command");
    }
    store_rule(&stored)?;
    Ok(stored)
}

/// Trim a tool result down to what is shown to the model.
pub fn sample_of(text: &str) -> String {
    if text.chars().count() <= SAMPLE_CHARS {
        return text.to_string();
    }
    let head: String = text.chars().take(SAMPLE_CHARS).collect();
    format!("{head}\n[... truncated for the sample ...]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_rule_out_of_prose() {
        let answer = r#"Sure! Here is the rule:
        {"tool": "Bash", "command_pattern": "npm run build", "match_type": "startswith",
         "transforms": [{"type": "remove_lines", "pattern": "^npm "}]}
        Hope that helps."#;
        let rule = parse_rule(answer).expect("a rule");
        assert_eq!(rule["tool"], json!("Bash"));
        assert_eq!(rule["transforms"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn rejects_a_rule_that_drops_the_failure() {
        let sample = "ok\nok\nERROR: it broke\nok\n";
        let rule = json!({
            "tool": "Bash",
            "command_pattern": "x",
            "transforms": [{"type": "remove_lines", "pattern": ".*"}],
        });
        let err = validate_rule(&rule, sample).expect_err("must be rejected");
        assert!(
            err.contains("failure line") || err.contains("empties the output"),
            "{err}"
        );
    }

    #[test]
    fn rejects_a_rule_that_does_not_shrink() {
        let sample = "hello world\n";
        let rule = json!({
            "tool": "Bash",
            "command_pattern": "x",
            "transforms": [{"type": "strip_noise"}],
        });
        let err = validate_rule(&rule, sample).expect_err("must be rejected");
        assert!(err.contains("shrink") || err.contains("saves too little"), "{err}");
    }

    #[test]
    fn accepts_a_rule_that_keeps_the_failure() {
        let sample = (0..50)
            .map(|i| format!("progress line {i}"))
            .chain(["ERROR: it broke".to_string()])
            .collect::<Vec<_>>()
            .join("\n");
        let rule = json!({
            "tool": "Bash",
            "command_pattern": "make build",
            "transforms": [{"type": "remove_lines", "pattern": "^progress line"}],
        });
        let checked = validate_rule(&rule, &sample).expect("must be accepted");
        assert_eq!(checked["command_pattern"], json!("make build"));
    }

    #[test]
    fn rejects_an_unknown_transform() {
        let rule = json!({
            "tool": "Bash",
            "command_pattern": "x",
            "transforms": [{"type": "run_shell", "pattern": "rm -rf /"}],
        });
        let err = validate_rule(&rule, "hello\n").expect_err("must be rejected");
        assert!(err.contains("unknown transform"), "{err}");
    }

    #[test]
    fn rejects_a_pattern_that_does_not_compile() {
        let rule = json!({
            "tool": "Bash",
            "command_pattern": "x",
            "transforms": [{"type": "remove_lines", "pattern": "([a-z"}],
        });
        let err = validate_rule(&rule, "hello\n").expect_err("must be rejected");
        assert!(err.contains("does not compile"), "{err}");
    }
}
