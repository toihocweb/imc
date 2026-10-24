//! Port of `main/proxy/services/copilot/context_filters.py` — tool-result
//! noise reduction.
//!
//! Strips terminal noise from large `tool_result` payloads (test/CI logs,
//! build output) while preserving meaningful content: test pass/fail lines,
//! summaries, real errors. Reduces tokens by collapsing repeated log lines and
//! capping oversized inline JSON/state dumps.
//!
//! ## Port notes
//!
//! * Python leans on `re` features the `regex` crate lacks — backreferences,
//!   lookbehind and lookahead. Those three patterns are hand-implemented and
//!   named inline: [`is_separator_line`], [`squish_multi_space`], and the two
//!   markdown-section helpers. Every other pattern is ported verbatim.
//! * Python's `len()` counts characters, so thresholds use `chars().count()`
//!   rather than byte length.
//! * The two config files (`context_filters_config.json`,
//!   `tool_result_filters.json`) are cached by mtime in process-local
//!   [`Mutex`]es, mirroring the module-level globals.
//! * Python's `re.sub` replacement syntax is `\1`/`\g<1>`; the `regex` crate
//!   uses `$1`, so literal `$` in a replacement is escaped before substitution.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use imc_proxy_types::types::anthropic::{
    AnthropicAssistantContentBlock, AnthropicInputMessage, AnthropicMessagesPayload,
    AnthropicToolResultContentBlock, AnthropicUserContentBlock, StringOrBlocks,
};
use regex::{Regex, RegexBuilder};
use serde_json::Value;

use crate::non_stream_translation::{
    assistant_content_block_view, tool_result_content_block_view, user_content_block_view,
    ContentBlock,
};

// ---------------------------------------------------------------------------
// File inspection and content classification helpers
// ---------------------------------------------------------------------------

fn clean_target_path(path_or_token: &str) -> &str {
    let p = path_or_token.trim().trim_matches(|c| c == '\'' || c == '"' || c == '`');
    let p = if let Some((base, _)) = p.split_once('#') {
        base
    } else {
        p
    };
    if let Some((base, suffix)) = p.rsplit_once(':') {
        if suffix.chars().all(|c| c.is_ascii_digit()) {
            base
        } else {
            p
        }
    } else {
        p
    }
}

/// Returns true if a string ends with a markdown extension (ignoring quotes and casing).
pub fn is_markdown_extension(path_or_token: &str) -> bool {
    let p = clean_target_path(path_or_token);
    let lower = p.to_lowercase();
    lower.ends_with(".md")
        || lower.ends_with(".markdown")
        || lower.ends_with(".mdx")
        || lower.ends_with(".mdown")
        || lower.ends_with(".mkdn")
}

/// Returns true if a path or token corresponds to any document, config, schema, or source code file.
pub fn is_document_or_source_extension(path_or_token: &str) -> bool {
    let p = clean_target_path(path_or_token);
    let lower = p.to_lowercase();
    let file_name = lower.split('/').last().unwrap_or(&lower);

    // Exact common filenames without standard extensions
    if matches!(
        file_name,
        "dockerfile"
            | "makefile"
            | "license"
            | "changelog"
            | "authors"
            | "notice"
            | "readme"
            | "gemfile"
            | "procfile"
            | "vagrantfile"
            | ".env"
            | ".gitignore"
            | ".dockerignore"
            | ".editorconfig"
    ) {
        return true;
    }

    // Markdown & Documentation & Text
    if is_markdown_extension(path_or_token)
        || lower.ends_with(".txt")
        || lower.ends_with(".text")
        || lower.ends_with(".rst")
        || lower.ends_with(".adoc")
        || lower.ends_with(".asciidoc")
        || lower.ends_with(".org")
    {
        return true;
    }

    // Structured config, data & schemas
    if lower.ends_with(".json")
        || lower.ends_with(".jsonl")
        || lower.ends_with(".yaml")
        || lower.ends_with(".yml")
        || lower.ends_with(".toml")
        || lower.ends_with(".xml")
        || lower.ends_with(".html")
        || lower.ends_with(".htm")
        || lower.ends_with(".css")
        || lower.ends_with(".scss")
        || lower.ends_with(".sass")
        || lower.ends_with(".less")
        || lower.ends_with(".csv")
        || lower.ends_with(".tsv")
        || lower.ends_with(".sql")
        || lower.ends_with(".env")
        || lower.ends_with(".ini")
        || lower.ends_with(".conf")
        || lower.ends_with(".config")
        || lower.ends_with(".properties")
        || lower.ends_with(".proto")
        || lower.ends_with(".graphql")
        || lower.ends_with(".gql")
    {
        return true;
    }

    // Source code
    if lower.ends_with(".rs")
        || lower.ends_with(".py")
        || lower.ends_with(".js")
        || lower.ends_with(".mjs")
        || lower.ends_with(".cjs")
        || lower.ends_with(".ts")
        || lower.ends_with(".mts")
        || lower.ends_with(".cts")
        || lower.ends_with(".tsx")
        || lower.ends_with(".jsx")
        || lower.ends_with(".go")
        || lower.ends_with(".java")
        || lower.ends_with(".c")
        || lower.ends_with(".h")
        || lower.ends_with(".cpp")
        || lower.ends_with(".cc")
        || lower.ends_with(".cxx")
        || lower.ends_with(".hpp")
        || lower.ends_with(".cs")
        || lower.ends_with(".rb")
        || lower.ends_with(".php")
        || lower.ends_with(".swift")
        || lower.ends_with(".kt")
        || lower.ends_with(".kts")
        || lower.ends_with(".scala")
        || lower.ends_with(".sh")
        || lower.ends_with(".bash")
        || lower.ends_with(".zsh")
        || lower.ends_with(".lua")
        || lower.ends_with(".zig")
        || lower.ends_with(".dart")
    {
        return true;
    }

    false
}

/// Returns true if the command or tool is reading a file (source code, text, config, or data).
pub fn is_file_read(command: &str, tool_name: &str) -> bool {
    let tool_lower = tool_name.trim().to_lowercase();
    let is_read_tool = matches!(
        tool_lower.as_str(),
        "read_file"
            | "readfile"
            | "view_file"
            | "viewfile"
            | "read"
            | "view"
            | "cat"
            | "open_file"
            | "openfile"
            | "read_url_content"
            | "file_reader"
            | "get_file_contents"
            | "read_document"
            | "fetch_file"
            | "retrieve"
            | "imc_retrieve"
    );

    if is_read_tool {
        return true;
    }

    let cmd_lower = command.trim().to_lowercase();
    if cmd_lower.contains("retrieve") {
        return true;
    }

    if is_document_or_source_extension(command) {
        return true;
    }
    for part in cmd_lower.split([';', '&', '|']) {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        let tokens: Vec<&str> = p.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        let read_verbs = [
            "cat", "head", "tail", "bat", "batcat", "less", "more", "sed", "awk",
            "view", "read", "glow",
        ];
        let has_read_verb = read_verbs.contains(&tokens[0])
            || tokens[0].ends_with("/cat")
            || tokens[0].ends_with("/head")
            || tokens[0].ends_with("/tail")
            || tokens[0].ends_with("/bat")
            || tokens[0].ends_with("/view")
            || tokens[0].ends_with("/read");

        let has_file_arg = tokens.iter().skip(1).any(|t| is_document_or_source_extension(t));
        if has_file_arg && (has_read_verb || tool_name.is_empty()) {
            return true;
        }
    }

    false
}

/// Returns true if the command or tool is reading a markdown file.
pub fn is_markdown_read(command: &str, tool_name: &str) -> bool {
    let tool_lower = tool_name.trim().to_lowercase();
    let is_read_tool = matches!(
        tool_lower.as_str(),
        "read_file"
            | "readfile"
            | "view_file"
            | "viewfile"
            | "read"
            | "view"
            | "cat"
            | "open_file"
            | "openfile"
            | "read_url_content"
    );

    if is_markdown_extension(command) {
        return true;
    }

    let cmd_lower = command.trim().to_lowercase();
    for part in cmd_lower.split([';', '&', '|']) {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        let tokens: Vec<&str> = p.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        let read_verbs = [
            "cat", "head", "tail", "bat", "batcat", "less", "more", "sed", "awk",
            "view", "read", "glow",
        ];
        let has_read_verb = read_verbs.contains(&tokens[0])
            || tokens[0].ends_with("/cat")
            || tokens[0].ends_with("/head")
            || tokens[0].ends_with("/tail")
            || tokens[0].ends_with("/bat")
            || tokens[0].ends_with("/view")
            || tokens[0].ends_with("/read");

        let has_md_arg = tokens.iter().any(|t| is_markdown_extension(t));
        if has_md_arg && (has_read_verb || is_read_tool || tool_name.is_empty()) {
            return true;
        }
    }

    false
}

/// Heuristic check whether text content is a structured configuration (YAML, TOML, JSON, INI).
pub fn is_structured_config(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }

    if (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        if serde_json::from_str::<Value>(trimmed).is_ok() {
            return true;
        }
    }

    if trimmed.starts_with('<') && (trimmed.ends_with('>') || trimmed.contains("</")) {
        return true;
    }

    if trimmed.starts_with("---\n") || trimmed.starts_with("---\r\n") {
        return true;
    }

    let sample_lines: Vec<&str> = trimmed.lines().take(40).collect();
    if sample_lines.is_empty() {
        return false;
    }

    let mut kv_or_section_count = 0;
    let mut total_content_lines = 0;

    for line in &sample_lines {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') || l.starts_with("//") || l.starts_with(';') {
            continue;
        }
        total_content_lines += 1;
        if l.starts_with('[') && l.ends_with(']') && !l.contains(' ') {
            kv_or_section_count += 1;
            continue;
        }
        if let Some((key, _)) = l.split_once(':') {
            let k = key.trim();
            if !k.is_empty() && !k.contains(' ') && k.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.') {
                kv_or_section_count += 1;
                continue;
            }
        }
        if let Some((key, _)) = l.split_once('=') {
            let k = key.trim();
            if !k.is_empty() && !k.contains(' ') && k.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.') {
                kv_or_section_count += 1;
                continue;
            }
        }
    }

    total_content_lines >= 3 && kv_or_section_count * 2 >= total_content_lines
}

/// Heuristic check whether text content is tabular data (CSV, TSV, or formatted table).
pub fn is_tabular_data(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }

    let sample_lines: Vec<&str> = trimmed.lines().take(30).collect();
    if sample_lines.len() < 3 {
        return false;
    }

    let pipe_lines = sample_lines.iter().filter(|l| l.contains('|')).count();
    if pipe_lines >= 3 && pipe_lines * 2 >= sample_lines.len() {
        return true;
    }

    let comma_counts: Vec<usize> = sample_lines.iter().map(|l| l.matches(',').count()).collect();
    if let Some(&first_count) = comma_counts.first() {
        if first_count >= 2 && comma_counts.iter().filter(|&&c| c == first_count).count() * 2 >= sample_lines.len() {
            return true;
        }
    }

    let tab_counts: Vec<usize> = sample_lines.iter().map(|l| l.matches('\t').count()).collect();
    if let Some(&first_count) = tab_counts.first() {
        if first_count >= 2 && tab_counts.iter().filter(|&&c| c == first_count).count() * 2 >= sample_lines.len() {
            return true;
        }
    }

    false
}

/// Heuristic check whether text is prose/documentation rather than build/test terminal output.
pub fn is_prose_or_documentation(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }

    let sample_lines: Vec<&str> = trimmed.lines().take(40).collect();
    if sample_lines.is_empty() {
        return false;
    }

    let has_log_markers = sample_lines.iter().any(|l| {
        let t = l.trim_start();
        t.starts_with("Compiling ")
            || t.starts_with("Finished ")
            || t.starts_with("Running ")
            || t.starts_with("test result:")
            || t.starts_with("--> ")
            || t.starts_with("=== ")
            || t.starts_with("FAILED ")
            || t.starts_with("PASS ")
            || t.starts_with("FAIL ")
            || t.starts_with("error: ")
            || t.starts_with("warning: ")
            || (t.starts_with('[') && (t.starts_with("[INFO]") || t.starts_with("[DEBUG]") || t.starts_with("[ERROR]") || t.starts_with("[WARN]")))
    });

    if has_log_markers {
        return false;
    }

    let sentence_ends = trimmed.matches(". ").count()
        + trimmed.matches(".\n").count()
        + trimmed.matches("? ").count()
        + trimmed.matches("!\n").count();

    sentence_ends >= 4
}

/// Heuristic check whether text content is a markdown document.
pub fn is_markdown_content(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }

    // YAML frontmatter check: starts with --- and contains closing ---
    if (trimmed.starts_with("---\n") || trimmed.starts_with("---\r\n"))
        && (trimmed[4..].contains("\n---\n") || trimmed[4..].contains("\r\n---\r\n"))
    {
        return true;
    }

    let mut heading_count = 0;
    let mut list_count = 0;
    let mut code_fence_count = 0;
    let mut md_link_count = 0;
    let mut table_count = 0;

    let sample_lines: Vec<&str> = trimmed.lines().take(60).collect();
    for line in &sample_lines {
        let l = line.trim();
        if l.starts_with("# ")
            || l.starts_with("## ")
            || l.starts_with("### ")
            || l.starts_with("#### ")
            || l.starts_with("##### ")
            || l.starts_with("###### ")
        {
            heading_count += 1;
        } else if l.starts_with("- ") || l.starts_with("* ") {
            list_count += 1;
        } else if l.starts_with("```") {
            code_fence_count += 1;
        } else if l.starts_with('|') && l.ends_with('|') && l.contains("---") {
            table_count += 1;
        }
        if l.contains("](") && l.contains('[') && l.contains(')') {
            md_link_count += 1;
        }
    }

    // Markdown documents have headings plus other structural markers
    if heading_count >= 2 {
        return true;
    }
    if heading_count >= 1
        && (trimmed.starts_with("# ")
            || trimmed.starts_with("## ")
            || list_count >= 1
            || code_fence_count >= 1
            || md_link_count >= 1
            || table_count >= 1)
    {
        return true;
    }
    if code_fence_count >= 2 || table_count >= 1 {
        return true;
    }

    false
}

/// Extract command, query, or file path from a tool use input map.
pub fn extract_command_or_path(input: &BTreeMap<String, Value>) -> String {
    for key in [
        "command",
        "CommandLine",
        "cmd",
        "query",
        "path",
        "file_path",
        "filePath",
        "TargetFile",
        "AbsolutePath",
        "file",
        "filename",
    ] {
        if let Some(Value::String(s)) = input.get(key) {
            return s.clone();
        }
    }
    String::new()
}

// ---------------------------------------------------------------------------
// Configuration paths
// ---------------------------------------------------------------------------

/// Port of `CONFIG_DIR` (`~/.indexmycode`).
fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".indexmycode")
}

/// Port of `CONTEXT_FILTERS_CONFIG_PATH`.
pub fn context_filters_config_path() -> PathBuf {
    config_dir().join("context_filters_config.json")
}

/// Port of `TOOL_RESULT_FILTERS_CONFIG_PATH`.
pub fn tool_result_filters_config_path() -> PathBuf {
    config_dir().join("tool_result_filters.json")
}

/// Port of `DEFAULT_TOOL_RESULT_FILTERS` — the built-in optimizers for the
/// three noisiest command families, persisted on first run.
///
/// Stored as a JSON literal rather than a `Vec<Rule>` struct so the on-disk
/// format and the rule *schema* stay user-editable exactly as in Python.
pub fn default_tool_result_filters() -> Vec<Value> {
    serde_json::from_str(DEFAULT_FILTERS_JSON).expect("DEFAULT_FILTERS_JSON is valid JSON")
}

const DEFAULT_FILTERS_JSON: &str = r#"[
    {
        "id": "rule_cypress_run",
        "name": "Cypress Test Optimizer",
        "description": "Keep passing/failing specs, assertions, errors, and summary (-93% tokens)",
        "enabled": true,
        "tool": "Bash",
        "command_pattern": "cypress run|cy:run-ct",
        "match_type": "regex",
        "transforms": [
            { "type": "strip_noise" },
            {
                "type": "keep_lines",
                "pattern": "(?i)(passing|failing|[✓✗×]|AssertionError|Error:|CypressError|Spec Ran:|Tests:|Passing:|Failing:|All specs passed!)"
            },
            {
                "type": "truncate_lines",
                "direction": "tail",
                "max_lines": 20
            }
        ]
    },
    {
        "id": "rule_mvn_test",
        "name": "Maven / Mvnd Test Optimizer",
        "description": "Strip ByteBuddy/Mockito JVM warnings, build plugin lifecycles, and passing test noise (-97% tokens)",
        "enabled": true,
        "tool": "Bash",
        "command_pattern": "mvn.*test",
        "match_type": "regex",
        "transforms": [
            { "type": "strip_noise" },
            {
                "type": "remove_lines",
                "pattern": "(Mockito is currently|Java agent has been loaded|EnableDynamicAgentLoading|Dynamic loading of agents|bootstrap classpath has been appended|JDK\\.instrument\\.traceUsage)"
            },
            {
                "type": "remove_lines",
                "pattern": "^\\s*\\[INFO\\]\\s*(--- (jacoco|resources|compiler|build-helper|surefire).*|argLine set|Using '.*'|Using auto detected|skip non existing|Nothing to compile|Test Source directory|Loading execution data|Analyzed bundle|All coverage checks)"
            },
            {
                "type": "remove_lines",
                "pattern": "^\\s*\\[INFO\\]\\s*(Running\\s+com\\..*|Tests run:\\s*\\d+,\\s*Failures:\\s*0,\\s*Errors:\\s*0,\\s*Skipped:\\s*0,\\s*Time elapsed:.*)"
            },
            {
                "type": "remove_lines",
                "pattern": "^\\s*\\[INFO\\]\\s*[-=]*\\s*$"
            },
            {
                "type": "remove_lines",
                "pattern": "^\\s*\\[imc filter\\] collapsed .*"
            },
            {
                "type": "regex_replace",
                "pattern": "\\n{3,}",
                "replacement": "\n\n",
                "flags": "m"
            }
        ]
    },
    {
        "id": "rule_gh_run",
        "name": "GitHub Actions Log Optimizer",
        "description": "Strip GitHub Actions step prefixes, runner script echo, and environment variable dumps (-96% tokens)",
        "enabled": true,
        "tool": "Bash",
        "command_pattern": "gh run",
        "match_type": "startswith",
        "transforms": [
            { "type": "strip_noise" },
            {
                "type": "regex_replace",
                "pattern": "^[^\\t\\r\\n]+\\t[^\\t\\r\\n]+(?:\\t(?:\\d{4}-\\d{2}-\\d{2}T\\S*\\s*)?)?",
                "replacement": "",
                "flags": "m"
            },
            {
                "type": "regex_replace",
                "pattern": "(?s)(?:##\\[group\\]Run|shell:\\s*\\S+|env:).*?##\\[endgroup\\]",
                "replacement": "",
                "flags": "m"
            },
            {
                "type": "remove_lines",
                "pattern": "^\\s*(?:PYEOF|RESPONSE=.*|rm -f\\s+.*|echo\\s+.*|if\\s+echo.*|elif\\s+echo.*|else$|fi$|exit\\s+\\d+$|##\\[(group|endgroup|debug)\\])"
            },
            {
                "type": "remove_lines",
                "pattern": "^\\s*$"
            },
            {
                "type": "regex_replace",
                "pattern": "\\n{3,}",
                "replacement": "\n\n",
                "flags": "m"
            }
        ]
    }
]"#;

// ---------------------------------------------------------------------------
// Config load / save, cached by mtime
// ---------------------------------------------------------------------------

/// Port of the `_tool_result_filters_cache` / `_tool_result_filters_mtime`
/// globals.
static TOOL_RESULT_FILTERS_CACHE: Mutex<(Option<SystemTime>, Option<Vec<Value>>)> =
    Mutex::new((None, None));

/// Where the learned rules are kept.
///
/// The user's own rules live in `tool_result_filters.json`; the rules a
/// model learned live in their own file so that editing the user's file
/// never loses them.
pub fn learned_filters_path() -> PathBuf {
    // The override exists so a test never writes into the real home.
    if let Ok(p) = std::env::var("IMC_LEARNED_FILTERS_PATH") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    config_dir().join("learned_filters.json")
}

/// Load the rules a model learned.
///
/// Read fresh on every call: the file is tiny, it is written by a background
/// task, and a rule that is one request late is better than a rule that
/// never arrives.
pub fn load_learned_filters() -> Vec<Value> {
    let path = learned_filters_path();
    read_json_file(&path)
        .and_then(|d| d.as_array().cloned())
        .unwrap_or_default()
}


/// A tool result the learning pass may look at.
#[derive(Debug, Clone)]
pub struct FilterObservation {
    /// The tool that produced it, e.g. `Bash`.
    pub tool: String,
    /// The command that was run.
    pub command: String,
    /// How many characters the result has.
    pub chars: usize,
    /// The head of the result.
    pub sample: String,
    /// Whether a rule already matched it.
    pub matched_rule: bool,
}

/// The `tool_use_id` -> (tool name, command) index of a payload.
pub fn tool_use_index(payload: &AnthropicMessagesPayload) -> HashMap<String, (String, String)> {
    let mut tool_uses: HashMap<String, (String, String)> = HashMap::new();
    for msg in &payload.messages {
        match msg {
            AnthropicInputMessage::User(m) => {
                for block in &blocks_of_user(&m.content) {
                    if let ContentBlock::ToolUse { id, name, input } = block {
                        if !id.is_empty() {
                            let command = extract_command_or_path(input);
                            tool_uses.insert(id.clone(), (name.clone(), command));
                        }
                    }
                }
            }
            AnthropicInputMessage::Assistant(m) => {
                for block in &blocks_of_assistant(&m.content) {
                    if let ContentBlock::ToolUse { id, name, input } = block {
                        if !id.is_empty() {
                            let command = extract_command_or_path(input);
                            tool_uses.insert(id.clone(), (name.clone(), command));
                        }
                    }
                }
            }
            AnthropicInputMessage::System(_) => {}
        }
    }
    tool_uses
}

/// The tool results of a payload, before any filtering.
///
/// The learning pass needs the original text: after the filter has run, the
/// noise is already gone and the model would learn from the wrong thing.
pub fn collect_observations(payload: &AnthropicMessagesPayload) -> Vec<FilterObservation> {
    let tool_uses = tool_use_index(payload);
    let rules = load_tool_result_filters_config();
    let mut out = Vec::new();
    for msg in &payload.messages {
        let AnthropicInputMessage::User(m) = msg else {
            continue;
        };
        let StringOrBlocks::Blocks(blocks) = &m.content else {
            continue;
        };
        for block in blocks {
            let AnthropicUserContentBlock::ToolResult(b) = block else {
                continue;
            };
            let (tool, command) = tool_uses
                .get(&b.tool_use_id)
                .cloned()
                .unwrap_or_else(|| (String::new(), String::new()));
            let text = match &b.content {
                StringOrBlocks::Text(s) => s.clone(),
                StringOrBlocks::Blocks(sub) => sub
                    .iter()
                    .filter_map(|c| match tool_result_content_block_view(c) {
                        ContentBlock::Text(t) => Some(t.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            };
            if text.trim().is_empty()
                || is_file_read(&command, &tool)
                || is_code_or_structured_content(&text)
            {
                continue;
            }
            let matched = rules
                .iter()
                .any(|r| match_tool_result_rule(r, &command, &tool));
            out.push(FilterObservation {
                tool,
                command,
                chars: text.chars().count(),
                sample: text.chars().take(8_000).collect(),
                matched_rule: matched,
            });
        }
    }
    out
}


/// Put back the failure lines a learned rule removed.
///
/// The rule was written by a model from one sample of one output. On the next
/// run the output differs, and a pattern that looked like noise there can
/// match a real error here. The failure is the answer, so it goes back — with
/// a note, so the reader knows it was rescued rather than kept.
fn restore_failure_lines(before: &str, after: &str) -> String {
    let kept: std::collections::HashSet<&str> = after.lines().collect();
    let mut rescued: Vec<&str> = Vec::new();
    for line in before.lines() {
        if is_failure_line(line) && !kept.contains(line) && !rescued.contains(&line) {
            rescued.push(line);
        }
    }
    if rescued.is_empty() {
        return after.to_string();
    }
    let mut out = after.to_string();
    out.push_str("\n[imc filter] kept ");
    out.push_str(&rescued.len().to_string());
    out.push_str(" failure line(s) the learned rule would have dropped:\n");
    for line in rescued {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// A line that reports a failure.
pub fn is_failure_line(line: &str) -> bool {
    let l = line.to_lowercase();
    l.contains("error")
        || l.contains("fail")
        || l.contains("panic")
        || l.contains("exception")
        || l.contains("fatal")
}

/// Port of `load_tool_result_filters_config`.
///
/// Served from a cache keyed on file mtime; falls back to the built-in rules
/// (persisting them) when the file is missing or unreadable.
pub fn load_tool_result_filters_config() -> Vec<Value> {
    let path = tool_result_filters_config_path();
    let mtime = file_mtime(&path);

    let cached = TOOL_RESULT_FILTERS_CACHE
        .lock()
        .ok()
        .and_then(|g| match (&g.0, &g.1) {
            (Some(cached_mtime), Some(rules)) if Some(*cached_mtime) == mtime => {
                Some(rules.clone())
            }
            _ => None,
        });
    if let Some(rules) = cached {
        return with_learned(rules);
    }

    if mtime.is_some() {
        if let Some(rules) = read_json_file(&path).and_then(|d| d.as_array().cloned()) {
            if let Ok(mut g) = TOOL_RESULT_FILTERS_CACHE.lock() {
                *g = (mtime, Some(rules.clone()));
            }
            return with_learned(rules);
        }
    }

    // Fallback to default and persist if file does not exist.
    let rules = default_tool_result_filters();
    save_tool_result_filters_config(&rules);
    with_learned(rules)
}

/// Append the learned rules after the user's own, so a hand-written rule
/// always wins over a learned one.
fn with_learned(mut rules: Vec<Value>) -> Vec<Value> {
    let learned = load_learned_filters();
    if !learned.is_empty() {
        rules.extend(learned);
    }
    rules
}

/// Port of `save_tool_result_filters_config` — write-tmp-then-rename, so a
/// concurrent reader never observes a half-written rules file.
pub fn save_tool_result_filters_config(rules: &[Value]) -> bool {
    let path = tool_result_filters_config_path();
    let Ok(serialized) = serde_json::to_string_pretty(rules) else {
        return false;
    };
    if !write_atomic(&path, &serialized) {
        return false;
    }

    let mtime = file_mtime(&path);
    if let Ok(mut g) = TOOL_RESULT_FILTERS_CACHE.lock() {
        *g = (mtime, Some(rules.to_vec()));
    }
    true
}

/// Port of the `_context_filters_enabled_cache` / `_context_filters_mtime`
/// globals.
static CONTEXT_FILTERS_ENABLED_CACHE: Mutex<(Option<SystemTime>, Option<bool>)> =
    Mutex::new((None, None));

/// Port of `is_context_filters_enabled` — the user can disable all filtering
/// from the config file, in which case payloads pass through untouched.
pub fn is_context_filters_enabled() -> bool {
    let path = context_filters_config_path();
    let mtime = file_mtime(&path);

    let cached = CONTEXT_FILTERS_ENABLED_CACHE
        .lock()
        .ok()
        .and_then(|g| match (&g.0, &g.1) {
            (Some(cached_mtime), Some(enabled)) if Some(*cached_mtime) == mtime => {
                Some(*enabled)
            }
            _ => None,
        });
    if let Some(enabled) = cached {
        return enabled;
    }

    let mut enabled = true;
    if mtime.is_some() {
        if let Some(data) = read_json_file(&path) {
            enabled = data.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        }
    }

    if let Ok(mut g) = CONTEXT_FILTERS_ENABLED_CACHE.lock() {
        *g = (mtime, Some(enabled));
    }
    enabled
}

/// Port of `set_context_filters_enabled` — persists the toggle and primes the
/// cache so the next read is a hit.
pub fn set_context_filters_enabled(enabled: bool) -> bool {
    let path = context_filters_config_path();
    let serialized = serde_json::json!({ "enabled": enabled }).to_string();

    let mtime = if write_atomic(&path, &serialized) {
        file_mtime(&path)
    } else {
        None
    };

    if let Ok(mut g) = CONTEXT_FILTERS_ENABLED_CACHE.lock() {
        *g = (mtime, Some(enabled));
    }
    enabled
}

/// Python's `tmp = path + ".tmp"; write(tmp); os.replace(tmp, path)`.
fn write_atomic(path: &Path, contents: &str) -> bool {
    let tmp_path = PathBuf::from(format!("{}.tmp", path.display()));
    if fs::write(&tmp_path, contents).is_err() {
        return false;
    }
    fs::rename(&tmp_path, path).is_ok()
}

fn file_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

fn read_json_file(path: &Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

// ---------------------------------------------------------------------------
// Regexes
// ---------------------------------------------------------------------------

/// Applied only when content is at least this large (chars). Small results
/// pass through.
const TOOL_RESULT_MIN_CHARS: usize = 1500;
/// Per-line cap for oversized inline dumps (e.g. huge minified blobs).
const TOOL_RESULT_INLINE_CAP: usize = 4000;

/// Port of `_ANSI_RE` — ANSI color / cursor codes.
fn ansi_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").expect("valid"))
}

/// Port of `_LINENO_RE` — `cat -n` style line prefix.
fn lineno_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*\d+\t").expect("valid"))
}

/// Port of `_LINENO_COLON_RE` — the `^\s*\d+[\t|:]` test used to spot
/// line-numbered output.
fn lineno_colon_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*\d+[\t|:]").expect("valid"))
}

/// Port of `_TS_RE` (`10:29:14 AM`).
fn ts_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\d{1,2}:\d{2}:\d{2}\s?[AP]M").expect("valid"))
}

/// Port of `_HEXID_RE` — uuids / hashes / request ids.
fn hexid_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b[0-9a-fA-F-]{8,}\b").expect("valid"))
}

/// Port of `_MS_RE` (`(228ms)`).
fn ms_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\(\d+\s?ms\)").expect("valid"))
}

/// Port of `_BIGNUM_RE` — epoch timestamps etc.
fn bignum_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\d{10,}").expect("valid"))
}

/// Port of `_CONTINUATION_RE` — a short indented fragment wraps the previous
/// log line.
fn continuation_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s{2,}\S").expect("valid"))
}

/// Port of `_BOX_DRAWING_RE` — box-drawing characters (single/double line,
/// corners, tees, crosses).
fn box_drawing_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Python: [─-╿═-╬╔╗╚╝║╒-╫]+
        Regex::new(
            "[\u{2500}-\u{257f}\u{2550}-\u{256c}\u{2554}\u{2557}\u{255a}\u{255d}\u{2551}\u{2552}-\u{256b}]+",
        )
        .expect("valid")
    })
}

/// Port of `_UNICODE_SEP_RUN_RE` — runs of 3+ box/dash separators.
fn unicode_sep_run_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            "[\u{2500}\u{2501}\u{2550}\u{254c}\u{254d}\u{2012}\u{2013}\u{2014}\u{2015}]{3,}",
        )
        .expect("valid")
    })
}

/// Port of `_INVISIBLE_RE` — zero-width and invisible Unicode characters.
fn invisible_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            "[\u{200b}\u{200c}\u{200d}\u{200e}\u{200f}\u{feff}\u{ad}\u{2060}\u{ffa0}]+",
        )
        .expect("valid")
    })
}

/// Port of `_TRUNCATED_RE` — the inline-truncation marker, kept so a truncated
/// line never dedup-collides with its untruncated original.
fn truncated_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\s*…\[\+\d+ chars\]$").expect("valid"))
}

/// Port of `_MULTI_NEWLINE_RE`.
fn multi_newline_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\n{3,}").expect("valid"))
}

/// Port of `_SEPARATOR_LINE_RE`.
///
/// Python is `^([=\-*~#])\1{2,}\s*$`, which needs a backreference the `regex`
/// crate has no syntax for. Hand-implemented to the same effect: 3+ repeats of
/// one identical separator character, then only trailing whitespace.
pub fn is_separator_line(line: &str) -> bool {
    let trimmed = line.trim_end();
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !matches!(first, '=' | '-' | '*' | '~' | '#') {
        return false;
    }
    let mut count = 1;
    for c in chars {
        if c != first {
            return false;
        }
        count += 1;
    }
    count >= 3
}

/// Port of `_MULTI_SPACE_RE` (`(?<=\S) {2,}`), whose Python lookbehind the
/// `regex` crate has no syntax for.
///
/// Collapses runs of 2+ spaces that follow a non-whitespace character; leading
/// indent (preceded by whitespace or at the very start) is preserved, matching
/// the Python lookbehind.
pub fn squish_multi_space(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ' ' {
            let run_start = i;
            while i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
            let run_len = i - run_start;
            let preceded_by_nonspace = run_start > 0 && !chars[run_start - 1].is_whitespace();
            if run_len >= 2 && preceded_by_nonspace {
                out.push(' ');
            } else {
                for _ in 0..run_len {
                    out.push(' ');
                }
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------

/// Port of `_normalize_log_line`.
///
/// Volatile tokens are normalized so repeated log lines collapse to one key.
pub fn normalize_log_line(line: &str) -> String {
    let s = ts_re().replace_all(line, "TS").into_owned();
    let s = hexid_re().replace_all(&s, "ID").into_owned();
    let s = ms_re().replace_all(&s, "(Nms)").into_owned();
    let s = bignum_re().replace_all(&s, "N").into_owned();

    // Preserve truncated-line identity — strip the marker for normalization but
    // append a fixed tag so the key differs from any original full-length line.
    let s = if truncated_re().is_match(&s) {
        let stripped = truncated_re().replace_all(&s, "").into_owned();
        format!("{stripped} [TRUNC]")
    } else {
        s
    };
    s.trim().to_string()
}

/// Port of `_cap_inline` — truncate a single oversized line, noting how much
/// was dropped. Python's `line[:cap]` is a character slice.
pub fn cap_inline(line: &str, cap: usize) -> String {
    let total = line.chars().count();
    if total <= cap {
        return line.to_string();
    }
    let kept: String = line.chars().take(cap).collect();
    format!("{kept} …[+{} chars]", total - cap)
}

/// Port of `_clean_line` — character-level cleaning of one line, after the
/// ANSI/lineno strip.
///
/// Order matters: invisible chars first, then box-drawing (replaced by a space
/// to avoid word-merging), then divider runs, then internal squish.
pub fn clean_line(line: &str) -> String {
    let s = invisible_re().replace_all(line, "").into_owned();
    let s = box_drawing_re().replace_all(&s, " ").into_owned();
    let s = unicode_sep_run_re().replace_all(&s, " ").into_owned();
    let s = squish_multi_space(&s);
    s.trim_end().to_string()
}

/// Port of `_is_code_or_structured_content`.
///
/// Detect if tool_result is source code, file contents, diff, or structured
/// code. Source code must NEVER have its lines folded, line numbers stripped,
/// or lines truncated.
pub fn is_code_or_structured_content(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }

    // Markdown documents should NEVER be filtered, truncated, or stripped of headings/dividers.
    if is_markdown_content(text) {
        return true;
    }

    // Structured configuration (JSON, YAML, TOML, XML, HTML, INI)
    if is_structured_config(text) {
        return true;
    }

    // Tabular data (tables, CSV, TSV)
    if is_tabular_data(text) {
        return true;
    }

    // Natural prose or documentation without log signatures
    if is_prose_or_documentation(text) {
        return true;
    }

    // Explicit IMC output markers.
    const IMC_MARKERS: [&str; 7] = [
        "[imc find]",
        "[imc symbols]",
        "[imc search]",
        "[imc filter]",
        "[imc-read]",
        "[related]",
        "[symbols — not expanded]",
    ];
    if IMC_MARKERS.iter().any(|m| text.trim_start().starts_with(m)) {
        return true;
    }

    // Git diffs and patch outputs.
    const DIFF_MARKERS: [&str; 4] = ["diff --git", "--- a/", "+++ b/", "@@ -"];
    if DIFF_MARKERS.iter().any(|m| text.starts_with(m)) {
        return true;
    }

    // Python `text.split("\n", 60)` keeps at most 61 parts.
    let lines: Vec<&str> = text.splitn(61, '\n').collect();
    let sample = lines
        .iter()
        .take(60)
        .copied()
        .collect::<Vec<&str>>()
        .join("\n");

    // Line-numbered output (Read tool, cat -n, grep -n, imc-read).
    let lineno_count = lines
        .iter()
        .filter(|l| lineno_colon_re().is_match(l))
        .count();
    if lineno_count >= 2 {
        return true;
    }

    // Common code syntax indicators.
    const CODE_INDICATORS: [&str; 38] = [
        "import ",
        "from ",
        "export ",
        "class ",
        "def ",
        "function",
        "const ",
        "let ",
        "var ",
        "public ",
        "private ",
        "protected ",
        "interface ",
        "type ",
        "enum ",
        "struct ",
        "namespace ",
        "package ",
        "#include ",
        "using ",
        "fn ",
        "impl ",
        "pub ",
        "func ",
        "return ",
        "<template",
        "<script",
        "<?php",
        "<!DOCTYPE",
        "SELECT ",
        "INSERT ",
        "UPDATE ",
        "@RestController",
        "@GetMapping",
        "@PostMapping",
        "@Service",
        "@Component",
        "@Autowired",
    ];
    let code_matches = CODE_INDICATORS
        .iter()
        .filter(|ind| sample.contains(**ind))
        .count();
    code_matches >= 2
}

/// Port of `remove_markdown_section`.
///
/// Removes the section whose heading matches `heading`, up to the next heading
/// of any level (or end of text), then re-collapses blank runs. Python builds
/// this from one regex with a trailing lookahead, which `regex` cannot
/// express, so the boundaries are found by scanning heading lines instead.
pub fn remove_markdown_section(text: &str, heading: &str) -> String {
    let h = heading.trim();
    if h.is_empty() {
        return text.to_string();
    }
    let clean_h = h.trim_start_matches('#').trim_start();
    if clean_h.is_empty() {
        return text.to_string();
    }

    let target = heading_re(clean_h);
    let lines: Vec<&str> = text.split('\n').collect();

    let mut out: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if target.is_match(lines[i]) {
            // Drop this heading line and its body, stopping at (not
            // including) the next heading line.
            i += 1;
            while i < lines.len() && !heading_start_re().is_match(lines[i]) {
                i += 1;
            }
            continue;
        }
        out.push(lines[i]);
        i += 1;
    }

    let joined = out.join("\n");
    multi_newline_re()
        .replace_all(&joined, "\n\n")
        .into_owned()
        .trim()
        .to_string()
}

/// Port of `keep_markdown_sections`.
///
/// Keeps the intro before the first markdown section plus any section whose
/// heading contains one of `headings`.
pub fn keep_markdown_sections(text: &str, headings: &[String]) -> String {
    if headings.is_empty() {
        return text.to_string();
    }

    let clean_headings: Vec<String> = headings
        .iter()
        .filter(|h| !h.trim().is_empty())
        .map(|h| h.trim_start_matches('#').trim().to_lowercase())
        .collect();

    // Python `pattern.split(text)` with one capture group yields
    // [intro, header, body, header, body, ...].
    let spans: Vec<(usize, usize)> = heading_line_re()
        .find_iter(text)
        .map(|m| (m.start(), m.end()))
        .collect();
    if spans.is_empty() {
        return text.to_string();
    }

    let mut parts: Vec<&str> = Vec::with_capacity(spans.len() * 2 + 1);
    parts.push(&text[..spans[0].0]);
    for (idx, &(start, end)) in spans.iter().enumerate() {
        parts.push(&text[start..end]);
        let body_end = spans.get(idx + 1).map_or(text.len(), |(next, _)| *next);
        parts.push(&text[end..body_end]);
    }

    // Python builds `out` as: the intro (when non-blank), then one entry per
    // matching section holding `header_line + body`. Keeping it a single list
    // of finished sections is what makes the "\n\n".join below correct even
    // when the intro was dropped.
    let intro = parts[0];
    let mut out: Vec<String> = Vec::new();
    if !intro.trim().is_empty() {
        out.push(intro.to_string());
    }

    let mut i = 1;
    while i < parts.len() {
        let header_line = parts[i];
        let body = if i + 1 < parts.len() { parts[i + 1] } else { "" };
        let header_title = header_line
            .trim()
            .trim_start_matches('#')
            .trim()
            .to_lowercase();
        if clean_headings
            .iter()
            .any(|h| header_title.contains(h.as_str()))
        {
            let mut section = String::with_capacity(header_line.len() + body.len());
            section.push_str(header_line);
            section.push_str(body);
            out.push(section);
        }
        i += 2;
    }

    out.join("\n\n").trim().to_string()
}

/// A line that begins a markdown heading (1-6 hashes then non-space text).
fn heading_start_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[ \t]*#{1,6}\s+\S").expect("valid"))
}

/// The `keep_markdown_sections` split pattern `(?m)(^[ \t]*#{1,6}\s+[^\n]+)`.
///
/// Python's `(?m)` matters: it is what makes `^` anchor at every line start
/// rather than only at offset 0, so this scans the whole document instead of
/// finding nothing unless the text opens with a heading.
fn heading_line_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        RegexBuilder::new(r"^[ \t]*#{1,6}\s+[^\n]+")
            .multi_line(true)
            .build()
            .expect("valid")
    })
}

/// A heading line whose title starts with `heading`.
fn heading_re(heading: &str) -> Regex {
    Regex::new(&format!(
        r"^[ \t]*#{{1,6}}\s+{}[^\n]*",
        regex::escape(heading)
    ))
    .expect("escaped heading is a valid pattern")
}

// ---------------------------------------------------------------------------
// Compiled-pattern cache (port of the `@lru_cache(maxsize=512)` decorator)
// ---------------------------------------------------------------------------

static COMPILED_REGEX_CACHE: Mutex<Option<HashMap<(String, u8), Regex>>> = Mutex::new(None);

/// Python `re.IGNORECASE`.
const RE_IGNORECASE: u8 = 1;
/// Python `re.MULTILINE`.
const RE_MULTILINE: u8 = 2;
/// Python `re.DOTALL`.
const RE_DOTALL: u8 = 4;

/// Port of `_get_compiled_regex` — compile once per (pattern, flags) pair.
fn get_compiled_regex(pattern_str: &str, flags: u8) -> Option<Regex> {
    let key = (pattern_str.to_string(), flags);

    if let Some(re) = COMPILED_REGEX_CACHE
        .lock()
        .ok()
        .and_then(|g| g.as_ref()?.get(&key).cloned())
    {
        return Some(re);
    }

    let built = RegexBuilder::new(pattern_str)
        .case_insensitive(flags & RE_IGNORECASE != 0)
        .multi_line(flags & RE_MULTILINE != 0)
        .dot_matches_new_line(flags & RE_DOTALL != 0)
        .build()
        .ok()?;

    if let Ok(mut g) = COMPILED_REGEX_CACHE.lock() {
        g.get_or_insert_with(HashMap::new)
            .insert(key, built.clone());
    }
    Some(built)
}

/// Port of `remove_matching_lines`.
pub fn remove_matching_lines(text: &str, pattern_str: &str) -> String {
    if pattern_str.is_empty() || text.is_empty() {
        return text.to_string();
    }
    let Some(rgx) = get_compiled_regex(pattern_str, RE_IGNORECASE) else {
        return text.to_string();
    };
    text.split('\n')
        .filter(|l| !rgx.is_match(l))
        .collect::<Vec<&str>>()
        .join("\n")
}

/// Port of `keep_matching_lines`.
pub fn keep_matching_lines(text: &str, pattern_str: &str) -> String {
    if pattern_str.is_empty() || text.is_empty() {
        return text.to_string();
    }
    let Some(rgx) = get_compiled_regex(pattern_str, RE_IGNORECASE) else {
        return text.to_string();
    };
    text.split('\n')
        .filter(|l| rgx.is_match(l))
        .collect::<Vec<&str>>()
        .join("\n")
}

/// Port of `regex_replace_content`.
///
/// `flags_str` is Python's flag string (`"i"`, `"m"`, `"s"`). The replacement
/// is literal, so `$` is escaped before handing it to the `regex` crate.
pub fn regex_replace_content(
    text: &str,
    pattern_str: &str,
    replacement: &str,
    flags_str: &str,
) -> String {
    if pattern_str.is_empty() || text.is_empty() {
        return text.to_string();
    }
    let flags_lower = flags_str.to_lowercase();
    let mut flags = 0u8;
    if flags_lower.contains('i') {
        flags |= RE_IGNORECASE;
    }
    if flags_lower.contains('m') {
        flags |= RE_MULTILINE;
    }
    if flags_lower.contains('s') {
        flags |= RE_DOTALL;
    }

    let Some(rgx) = get_compiled_regex(pattern_str, flags) else {
        return text.to_string();
    };
    rgx.replace_all(text, replacement.replace('$', "$$"))
        .into_owned()
}

/// Port of `truncate_lines_content`.
///
/// `head_and_tail` / `both` keep a head and a tail slice around a marker;
/// `tail` keeps the last `max_lines`; the default `head` keeps the first.
pub fn truncate_lines_content(
    text: &str,
    max_lines: usize,
    direction: &str,
    head_lines: usize,
    tail_lines: usize,
) -> String {
    if text.is_empty() {
        return text.to_string();
    }
    let lines: Vec<&str> = text.lines().collect();
    let total_lines = lines.len();

    match direction {
        "head_and_tail" | "both" => {
            let needed = head_lines + tail_lines;
            if total_lines <= needed {
                return text.to_string();
            }
            let dropped = total_lines - needed;
            format!(
                "{}\n\n[... {dropped} lines collapsed (head & tail preserved) ...]\n\n{}",
                slice_head(&lines, head_lines),
                slice_tail(&lines, tail_lines),
            )
        }
        "tail" => {
            if total_lines <= max_lines {
                return text.to_string();
            }
            let dropped = total_lines - max_lines;
            format!(
                "[... {dropped} lines truncated ...]\n{}",
                slice_tail(&lines, max_lines)
            )
        }
        _ => {
            // "head" default
            if total_lines <= max_lines {
                return text.to_string();
            }
            let dropped = total_lines - max_lines;
            format!(
                "{}\n[... {dropped} lines truncated ...]",
                slice_head(&lines, max_lines)
            )
        }
    }
}

/// Python `lines[:n]`.
fn slice_head(lines: &[&str], n: usize) -> String {
    lines.iter().take(n).copied().collect::<Vec<&str>>().join("\n")
}

/// Python `lines[-n:]` — including its `-0` quirk of yielding the whole list.
fn slice_tail(lines: &[&str], n: usize) -> String {
    if n == 0 {
        return lines.join("\n");
    }
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// Port of `_clean_terminal_noise_only` — the `strip_noise` transform.

// ---------------------------------------------------------------------------
// Repeated success lines
// ---------------------------------------------------------------------------

/// Port of `_SUCCESS_MARKER_RE` — a line that ends in a success marker.
///
/// The `regex` crate has no `\b` inside a `(?i)` alternation problem, so the
/// word boundary is spelled `\b` the same way Python does.
fn success_marker_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        RegexBuilder::new(r"(?i)\b(PASSED|PASS|OK|SUCCESS)\s*$|[\u{2713}\u{2714}]\s*$")
            .build()
            .expect("valid success marker regex")
    })
}

/// Below this, a group of success lines is a coincidence, not a pattern.
const MIN_SUCCESS_GROUP: usize = 3;

/// Port of `_success_group_key` — the group a success line belongs to.
///
/// Only lines that start with something path-like are grouped: a bare "ok"
/// says nothing about what it refers to. A pytest line
/// `tests/test_api.py::test_login` groups under `tests/test_api.py`.
pub fn success_group_key(line: &str) -> Option<String> {
    if !success_marker_re().is_match(line) {
        return None;
    }
    let first = line.split_whitespace().next()?.trim();
    if !first.contains('/') && !first.contains('.') {
        return None;
    }
    // pytest: "tests/test_api.py::test_login" -> "tests/test_api.py"
    let first = match first.rsplit_once("::") {
        Some((head, _)) => head,
        None => first,
    };
    Some(first.to_string())
}

/// Port of `_collapse_success_lines` — collapse runs of per-item success lines
/// that share a path.
///
/// A test run prints one line per test, and every line differs, so the
/// duplicate collapse cannot touch them: a 400-test run costs 400 lines of
/// context while the failures and the summary are buried in the middle. A
/// group of three or more collapses to its first line plus a count. Failures
/// are never collapsed — the failure is the answer.
pub fn collapse_success_lines(lines: &[String]) -> (Vec<String>, usize) {
    let keys: Vec<Option<String>> = lines.iter().map(|l| success_group_key(l)).collect();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for k in keys.iter().flatten() {
        *counts.entry(k.clone()).or_insert(0) += 1;
    }

    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut collapsed = 0usize;
    for (line, key) in lines.iter().zip(keys.iter()) {
        let Some(key) = key else {
            out.push(line.clone());
            continue;
        };
        let count = counts.get(key).copied().unwrap_or(0);
        if count < MIN_SUCCESS_GROUP {
            out.push(line.clone());
            continue;
        }
        if !emitted.insert(key.clone()) {
            collapsed += 1;
            continue;
        }
        out.push(line.clone());
        out.push(format!(
            "[imc filter] collapsed {} more success line(s) for {key}",
            count - 1
        ));
    }
    (out, collapsed)
}

fn clean_terminal_noise_only(text: &str) -> String {
    text.split('\n')
        .map(|l| clean_line(&lineno_re().replace_all(&ansi_re().replace_all(l, ""), "")))
        .filter(|l| !is_separator_line(l))
        .collect::<Vec<String>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Transform pipeline
// ---------------------------------------------------------------------------

/// Port of `execute_transforms_pipeline` — dispatch on each transform's
/// `type`, applied in order.
pub fn execute_transforms_pipeline(text: &str, transforms: Option<&Vec<Value>>) -> String {
    let mut current = text.to_string();

    let Some(transforms) = transforms else {
        return current;
    };

    for tr in transforms {
        let t_type = tr.get("type").and_then(Value::as_str).unwrap_or("");

        match t_type {
            "remove_section" => {
                let heading = tr.get("heading").and_then(Value::as_str).unwrap_or("");
                current = remove_markdown_section(&current, heading);
            }
            "keep_sections" => {
                let headings = match tr.get("headings") {
                    Some(Value::Array(items)) => items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|s| s.to_string())
                        .collect(),
                    Some(Value::String(s)) => s
                        .split(',')
                        .map(|h| h.trim().to_string())
                        .filter(|h| !h.is_empty())
                        .collect(),
                    _ => Vec::new(),
                };
                current = keep_markdown_sections(&current, &headings);
            }
            "remove_lines" => {
                let pattern = tr.get("pattern").and_then(Value::as_str).unwrap_or("");
                current = remove_matching_lines(&current, pattern);
            }
            "keep_lines" => {
                let pattern = tr.get("pattern").and_then(Value::as_str).unwrap_or("");
                current = keep_matching_lines(&current, pattern);
            }
            "regex_replace" => {
                let pattern = tr.get("pattern").and_then(Value::as_str).unwrap_or("");
                let replacement = tr.get("replacement").and_then(Value::as_str).unwrap_or("");
                let flags = tr.get("flags").and_then(Value::as_str).unwrap_or("m");
                current = regex_replace_content(&current, pattern, replacement, flags);
            }
            "truncate_lines" => {
                // Python wraps each `int()` in try/except, defaulting on any
                // non-numeric input.
                let max_l = tr.get("max_lines").and_then(value_to_usize).unwrap_or(100);
                let direction = tr.get("direction").and_then(Value::as_str).unwrap_or("head");
                let h_lines = tr
                    .get("head_lines")
                    .and_then(value_to_usize)
                    .unwrap_or(20);
                let t_lines = tr
                    .get("tail_lines")
                    .and_then(value_to_usize)
                    .unwrap_or(50);
                current = truncate_lines_content(&current, max_l, direction, h_lines, t_lines);
            }
            "head_and_tail" | "head_tail" => {
                let h_lines = tr
                    .get("head_lines")
                    .and_then(value_to_usize)
                    .unwrap_or(20);
                let t_lines = tr
                    .get("tail_lines")
                    .and_then(value_to_usize)
                    .unwrap_or(50);
                current = truncate_lines_content(&current, 100, "head_and_tail", h_lines, t_lines);
            }
            "strip_noise" => {
                current = clean_terminal_noise_only(&current);
            }
            _ => {}
        }
    }

    current
}

/// Python's `int(x)` inside a `try/except (ValueError, TypeError)`.
fn value_to_usize(v: &Value) -> Option<usize> {
    match v {
        Value::Number(n) => n.as_i64().and_then(|i| usize::try_from(i).ok()),
        Value::String(s) => s.trim().parse::<usize>().ok(),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Rule matching
// ---------------------------------------------------------------------------

/// Port of `match_tool_result_rule`.
///
/// A rule applies when it is enabled, its `tool` matches the calling tool
/// (`*` is a wildcard), and its `command_pattern` matches the command under
/// the rule's `match_type`.
pub fn match_tool_result_rule(rule: &Value, command: &str, tool_name: &str) -> bool {
    if !rule.get("enabled").and_then(Value::as_bool).unwrap_or(true) {
        return false;
    }

    let rule_tool = rule
        .get("tool")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let t_clean = tool_name.trim().to_lowercase();
    if !rule_tool.is_empty() && rule_tool != "*" && rule_tool != t_clean {
        return false;
    }

    let pattern = rule
        .get("command_pattern")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if pattern.is_empty() || pattern == "*" {
        return true;
    }

    let cmd_clean = command.trim();
    let cmd_lower = cmd_clean.to_lowercase();
    let pat_lower = pattern.to_lowercase();
    let match_type = rule
        .get("match_type")
        .and_then(Value::as_str)
        .unwrap_or("contains")
        .to_lowercase();

    // Python splits an alternation list on either delimiter via
    // `pattern.replace("|", ",").split(",")`.
    let has_alternation = pattern.contains(',') || pattern.contains('|');
    let sub_patterns: Vec<String> = if has_alternation {
        pattern
            .replace('|', ",")
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        Vec::new()
    };
    let use_subs = has_alternation && !sub_patterns.is_empty();

    match match_type.as_str() {
        "exact" => {
            if use_subs {
                sub_patterns.iter().any(|p| *p == cmd_lower)
            } else {
                cmd_lower == pat_lower
            }
        }
        "startswith" => {
            if use_subs {
                sub_patterns.iter().any(|p| cmd_lower.starts_with(p.as_str()))
            } else {
                cmd_lower.starts_with(&pat_lower)
            }
        }
        "regex" => match get_compiled_regex(pattern, RE_IGNORECASE) {
            Some(rgx) => rgx.is_match(cmd_clean),
            None => false,
        },
        _ => {
            // "contains" default
            if use_subs {
                sub_patterns.iter().any(|p| cmd_lower.contains(p.as_str()))
            } else {
                cmd_lower.contains(&pat_lower)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Port of `filter_tool_result_content`.
///
/// Checks user-defined command rules first; falls back to default terminal
/// log de-duplication when no custom rule matched. The fallback only engages
/// on large, non-code payloads.
pub fn filter_tool_result_content(text: &str, command: &str, tool_name: &str) -> String {
    if text.is_empty() {
        return text.to_string();
    }

    // File reads (source code, text, configs, markdown, data) are NEVER filtered or truncated!
    if is_file_read(command, tool_name) {
        return text.to_string();
    }

    // Check custom user-defined command rules first.
    let rules = load_tool_result_filters_config();
    let matched_rules: Vec<&Value> = rules
        .iter()
        .filter(|r| match_tool_result_rule(r, command, tool_name))
        .collect();

    if !matched_rules.is_empty() {
        let mut out = text.to_string();
        for r in matched_rules {
            // A learned rule was written against one sample, so it can be
            // right about that sample and wrong about the next output of the
            // same command. The safety net: a rule a model wrote may only
            // remove noise, never a failure line. A hand-written rule is left
            // alone - the user knows what they want.
            let learned = r.get("source").and_then(Value::as_str) == Some("learned");
            let before = out.clone();
            out = execute_transforms_pipeline(&out, r.get("transforms").and_then(Value::as_array));
            if learned {
                out = restore_failure_lines(&before, &out);
            }
        }
        return out;
    }

    if text.chars().count() < TOOL_RESULT_MIN_CHARS {
        return text.to_string();
    }

    // NEVER filter, fold, strip, or truncate source code or file read outputs!
    if is_code_or_structured_content(text) {
        return text.to_string();
    }

    // Step 1: per-line ANSI + line-number strip, then character cleanup.
    let raw_lines: Vec<String> = text
        .split('\n')
        .map(|l| clean_line(&lineno_re().replace_all(&ansi_re().replace_all(l, ""), "")))
        .collect();

    // Step 1c: drop pure ASCII separator lines (===, ---, ***, ~~~, ###).
    let raw_lines: Vec<String> = raw_lines
        .into_iter()
        .filter(|l| !is_separator_line(l))
        .collect();

    // Step 2: fold continuation fragments into the line above.
    let mut folded: Vec<String> = Vec::with_capacity(raw_lines.len());
    for line in raw_lines {
        let should_fold = !folded.is_empty()
            && !line.is_empty()
            && continuation_re().is_match(&line)
            && !folded.last().map(|l| l.trim().is_empty()).unwrap_or(true);
        if should_fold {
            if let Some(last) = folded.last_mut() {
                last.push(' ');
                last.push_str(line.trim());
            }
        } else {
            folded.push(line);
        }
    }

    // Step 2b: collapse per-item success lines (one line per test).
    let (folded, success_dropped) = collapse_success_lines(&folded);

    // Steps 3 + 4: dedup + inline cap.
    let mut out: Vec<String> = Vec::with_capacity(folded.len());
    let mut out_keys: Vec<String> = Vec::with_capacity(folded.len());
    let mut seen_counts: HashMap<String, i64> = HashMap::new();
    let mut dropped: i64 = success_dropped as i64;

    for line in folded {
        let key = normalize_log_line(&line);

        // Consecutive duplicate (any length).
        if !key.is_empty() && out_keys.last().map(|k| *k == key).unwrap_or(false) {
            dropped += 1;
            continue;
        }

        // Global dedup for substantial lines only (keep short structural
        // lines).
        if !key.is_empty() && key.chars().count() > 40 {
            let count = seen_counts.entry(key.clone()).or_insert(0);
            let prior = *count;
            *count += 1;
            if prior >= 1 {
                dropped += 1;
                continue;
            }
        }

        out.push(cap_inline(&line, TOOL_RESULT_INLINE_CAP));
        out_keys.push(key);
    }

    let mut result = multi_newline_re()
        .replace_all(&out.join("\n"), "\n\n")
        .into_owned();
    if dropped > 0 {
        result.push_str(&format!(
            "\n[imc filter] collapsed {dropped} duplicate/noise log line(s)"
        ));
    }

    // Guard: never return something larger than the input.
    if result.chars().count() < text.chars().count() {
        result
    } else {
        text.to_string()
    }
}

/// Port of `_approx_tokens` — ~4 chars/token heuristic.
pub fn approx_tokens(chars: usize) -> usize {
    chars / 4
}

/// Port of `_payload_text_chars` — total characters of all filterable text:
/// system text blocks plus message text / tool_result content.
pub fn payload_text_chars(payload: &AnthropicMessagesPayload) -> usize {
    let mut total = 0usize;

    if let Some(system) = payload.system.as_ref() {
        match system {
            StringOrBlocks::Text(s) => total += s.chars().count(),
            StringOrBlocks::Blocks(blocks) => {
                for block in blocks {
                    total += block.text.chars().count();
                }
            }
        }
    }

    for msg in &payload.messages {
        total += message_filterable_chars(msg);
    }

    total
}

/// Per-message slice of [`payload_text_chars`].
fn message_filterable_chars(msg: &AnthropicInputMessage) -> usize {
    match msg {
        AnthropicInputMessage::User(m) => match &m.content {
            StringOrBlocks::Text(s) => s.chars().count(),
            StringOrBlocks::Blocks(blocks) => blocks
                .iter()
                .map(|b| match user_content_block_view(b) {
                    ContentBlock::Text(t) => t.chars().count(),
                    ContentBlock::ToolResult { content, .. } => match content {
                        StringOrBlocks::Text(c) => c.chars().count(),
                        // Python only counts string tool_result content.
                        StringOrBlocks::Blocks(_) => 0,
                    },
                    _ => 0,
                })
                .sum(),
        },
        AnthropicInputMessage::Assistant(m) => match &m.content {
            StringOrBlocks::Text(s) => s.chars().count(),
            StringOrBlocks::Blocks(blocks) => blocks
                .iter()
                .map(|b| match assistant_content_block_view(b) {
                    ContentBlock::Text(t) => t.chars().count(),
                    _ => 0,
                })
                .sum(),
        },
        AnthropicInputMessage::System(m) => match &m.content {
            StringOrBlocks::Text(s) => s.chars().count(),
            StringOrBlocks::Blocks(blocks) => {
                blocks.iter().map(|b| b.text.chars().count()).sum()
            }
        },
    }
}

/// Port of `process_context_filters`.
///
/// Applies command output filters to message `tool_result` blocks in place.
/// `tool_use_id` is resolved against the whole conversation first, so a result
/// is filtered according to the command that produced it, wherever it sits.
///
/// Returns `true` when the payload was modified.
pub fn process_context_filters(payload: &mut AnthropicMessagesPayload) -> bool {
    if !is_context_filters_enabled() {
        return false;
    }

    let chars_before = payload_text_chars(payload);

    // Index tool_use calls across all messages to map tool_use_id -> (tool_name, command).
    let tool_uses = tool_use_index(payload);

    // Process tool_result blocks in messages.
    let mut modified = false;
    for msg in payload.messages.iter_mut() {
        match msg {
            AnthropicInputMessage::User(m) => filter_tool_result_blocks_user(&mut m.content, &tool_uses, &mut modified),
            AnthropicInputMessage::Assistant(m) => {
                filter_tool_result_blocks_assistant(&mut m.content, &tool_uses, &mut modified)
            }
            AnthropicInputMessage::System(_) => {}
        }
    }

    let chars_after = payload_text_chars(payload);
    let tok_before = approx_tokens(chars_before);
    let tok_after = approx_tokens(chars_after);
    let saved = tok_before.saturating_sub(tok_after);
    if saved > 0 {
        let pct = if tok_before > 0 {
            saved as f64 / tok_before as f64 * 100.0
        } else {
            0.0
        };
        // Python logs this at debug level; the crate has no logging facade
        // here, so the caller-visible signal is the returned `modified` flag.
        let _ = pct;
    }

    modified
}

/// The user-message block views used when indexing `tool_use` calls.
fn blocks_of_user(content: &StringOrBlocks<AnthropicUserContentBlock>) -> Vec<ContentBlock> {
    match content {
        StringOrBlocks::Text(_) => Vec::new(),
        StringOrBlocks::Blocks(blocks) => {
            blocks.iter().map(user_content_block_view).collect()
        }
    }
}

/// The assistant-message block views used when indexing `tool_use` calls.
fn blocks_of_assistant(
    content: &StringOrBlocks<AnthropicAssistantContentBlock>,
) -> Vec<ContentBlock> {
    match content {
        StringOrBlocks::Text(_) => Vec::new(),
        StringOrBlocks::Blocks(blocks) => {
            blocks.iter().map(assistant_content_block_view).collect()
        }
    }
}

/// Port of `_filter_tool_result_block` applied to a user message's blocks.
fn filter_tool_result_blocks_user(
    content: &mut StringOrBlocks<AnthropicUserContentBlock>,
    tool_uses: &HashMap<String, (String, String)>,
    modified: &mut bool,
) {
    let StringOrBlocks::Blocks(blocks) = content else {
        return;
    };
    for block in blocks.iter_mut() {
        if user_content_block_view(block).type_str() != "tool_result" {
            continue;
        }
        let AnthropicUserContentBlock::ToolResult(b) = block else {
            continue;
        };
        let (tool_name, command) = tool_uses
            .get(&b.tool_use_id)
            .cloned()
            .unwrap_or_else(|| (String::new(), String::new()));
        if let Some(filtered) = filter_tool_result_content_block(&b.content, &command, &tool_name) {
            b.content = filtered;
            *modified = true;
        }
    }
}

/// Port of `_filter_tool_result_block` applied to an assistant message's blocks.
///
/// Python runs the same `_process_str` over assistant blocks too; assistant
/// messages do not carry `tool_result` blocks, so this is a no-op guard kept
/// for structural parity with the Python loop.
fn filter_tool_result_blocks_assistant(
    content: &mut StringOrBlocks<AnthropicAssistantContentBlock>,
    _tool_uses: &HashMap<String, (String, String)>,
    _modified: &mut bool,
) {
    let StringOrBlocks::Blocks(blocks) = content else {
        return;
    };
    // Assistant blocks (text / tool_use / thinking) are never tool_result
    // blocks, so nothing to filter — but keep the loop shape so a future
    // block type is covered without reworking the call site.
    for block in blocks.iter_mut() {
        let _ = block;
    }
}

/// Port of `_filter_tool_result_block`'s inner `_process_str`.
///
/// String content is filtered whole; block-list content is filtered one text
/// block at a time. Returns `None` when nothing changed.
fn filter_tool_result_content_block(
    content: &StringOrBlocks<AnthropicToolResultContentBlock>,
    command: &str,
    tool_name: &str,
) -> Option<StringOrBlocks<AnthropicToolResultContentBlock>> {
    match content {
        StringOrBlocks::Text(s) => {
            if s.is_empty() {
                return None;
            }
            let filtered = filter_tool_result_content(s, command, tool_name);
            if filtered == *s {
                None
            } else {
                Some(StringOrBlocks::Text(filtered))
            }
        }
        StringOrBlocks::Blocks(blocks) => {
            let mut any_changed = false;
            let mut new_blocks: Vec<AnthropicToolResultContentBlock> = Vec::with_capacity(blocks.len());
            for b in blocks {
                // Only text sub-blocks are filterable; anything else passes
                // through untouched.
                let ContentBlock::Text(original) = tool_result_content_block_view(b) else {
                    new_blocks.push(b.clone());
                    continue;
                };
                if original.is_empty() {
                    new_blocks.push(b.clone());
                    continue;
                }
                let filtered = filter_tool_result_content(&original, command, tool_name);
                if filtered != original {
                    let mut clone = b.clone();
                    if let AnthropicToolResultContentBlock::Text(t) = &mut clone {
                        t.text = filtered;
                    }
                    new_blocks.push(clone);
                    any_changed = true;
                    continue;
                }
                new_blocks.push(b.clone());
            }
            if any_changed {
                Some(StringOrBlocks::Blocks(new_blocks))
            } else {
                None
            }
        }
    }
}
