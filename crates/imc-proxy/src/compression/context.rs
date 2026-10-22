//! Request-path compression.
//!
//! The proxy sits between Claude Code and the model, so it sees every tool
//! result the agent is about to re-read: a 20 KB file read, a full `cargo
//! build` log, a whole test suite's output. Most of that is not the answer.
//!
//! This module shrinks oversized tool results on the way through and leaves a
//! retrieval handle behind, so the model can pull the original back verbatim
//! when it actually needs the detail. Two rules keep it safe:
//!
//!   * nothing under [`MIN_TOKENS`] is touched — small results are already
//!     cheap and are usually exactly what the agent asked for;
//!   * nothing is replaced unless the compressor saves at least
//!     [`MIN_SAVINGS`], because a marker that costs more than it saves is
//!     worse than forwarding the original.
//!
//! Error results are never compressed: an exception message is the answer, not
//! context noise.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use imc_core::settings;
use imc_proxy_types::types::anthropic::{
    AnthropicAssistantContentBlock, AnthropicInputMessage, AnthropicToolResultContentBlock,
    AnthropicUserContentBlock, StringOrBlocks,
};
use serde_json::Value;

use crate::compression::{ccr_store::CcrStore, compress, kind_slug};

/// Tool results smaller than this are forwarded untouched (tokens, ≈4 B each).
pub const MIN_TOKENS: usize = 400;
/// Minimum fraction of tokens the compressor must save before we swap.
pub const MIN_SAVINGS: f64 = 0.20;

/// What a pass over the conversation did.
#[derive(Debug, Default, Clone, Copy)]
pub struct CompressionReport {
    /// Tool result blocks inspected.
    pub considered: usize,
    /// Blocks actually replaced.
    pub compressed: usize,
    pub original_tokens: usize,
    pub compressed_tokens: usize,
}

impl CompressionReport {
    pub fn saved_tokens(&self) -> usize {
        self.original_tokens.saturating_sub(self.compressed_tokens)
    }

    pub fn is_empty(&self) -> bool {
        self.compressed == 0
    }
}

/// Where the CCR store lives. Honours `IMC_CCR_DB` so tests and alternate
/// installs can point it somewhere else.
pub fn ccr_db_path() -> PathBuf {
    if let Some(p) = std::env::var_os("IMC_CCR_DB") {
        return PathBuf::from(p);
    }
    imc_core::settings::path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("proxy_ccr.db")
}

/// Extract command, query, or path from a tool use input map.
fn extract_command_or_query(input: &BTreeMap<String, Value>) -> Option<String> {
    for key in [
        "command",
        "CommandLine",
        "cmd",
        "query",
        "pattern",
        "path",
        "file_path",
        "filePath",
        "TargetFile",
        "AbsolutePath",
        "file",
        "filename",
        "search_term",
        "term",
        "prompt",
    ] {
        if let Some(Value::String(s)) = input.get(key) {
            return Some(s.clone());
        }
    }
    None
}

/// Returns true if tool name belongs to search, navigation, or retrieval.
pub fn is_bypass_tool_name(name: &str) -> bool {
    let exact_matches = [
        "grep",
        "ripgrep",
        "rg",
        "ag",
        "ack",
        "glob",
        "globtool",
        "find",
        "file_search",
        "search_files",
        "ls",
        "list_dir",
        "dir",
        "tree",
        "locate",
        "list_directory",
        "search",
        "code_search",
        "search_code",
        "workspace_search",
        "imc_search",
        "trace",
        "imc_trace",
        "memory",
        "memory_search",
        "imc_memory",
        "web_search",
        "search_web",
        "brave_search",
        "google_search",
        "git_status",
    ];

    if exact_matches.contains(&name) {
        return true;
    }

    // Namespaced / MCP tool names like "mcp__imc__search", "mcp__search_files"
    let parts: Vec<&str> = name
        .split(['_', '-', ':', '.'])
        .filter(|s| !s.is_empty())
        .collect();
    for part in parts {
        if matches!(
            part,
            "search" | "grep" | "ripgrep" | "rg" | "glob" | "find" | "trace" | "memory" | "ls"
        ) {
            return true;
        }
    }

    false
}

/// Returns true if tool is a generic shell / command execution tool.
fn is_exec_tool_name(name: &str) -> bool {
    matches!(
        name,
        "bash"
            | "sh"
            | "zsh"
            | "exec"
            | "run_command"
            | "terminal"
            | "command"
            | "execute_command"
            | "cli"
    )
}

/// Returns true if command being run performs search, discovery, or status inspection.
pub fn is_bypass_command(cmd: &str) -> bool {
    let lower = cmd.trim().to_lowercase();
    for line in lower.lines() {
        let l = line.trim();
        for part in l.split([';', '&', '|']) {
            let p = part.trim();
            if p.is_empty() {
                continue;
            }

            // imc CLI inspection subcommands
            if p.starts_with("imc search")
                || p.starts_with("imc trace")
                || p.starts_with("imc memory")
                || p.starts_with("imc memory-search")
                || p.starts_with("imc context")
                || p.starts_with("imc ask")
                || p.contains("/imc search")
                || p.contains("/imc trace")
                || p.contains("/imc memory")
            {
                return true;
            }

            // grep / ripgrep
            if p.starts_with("grep ")
                || p.starts_with("rg ")
                || p.starts_with("ripgrep ")
                || p.starts_with("ag ")
                || p.starts_with("ack ")
                || p.contains("/grep ")
                || p.contains("/rg ")
            {
                return true;
            }

            // find / fd / glob
            if p.starts_with("find ")
                || p.starts_with("fd ")
                || p.starts_with("locate ")
                || p.contains("/find ")
                || p.contains("/fd ")
            {
                return true;
            }

            // ls / tree
            if p.starts_with("ls ")
                || p == "ls"
                || p.starts_with("tree ")
                || p == "tree"
            {
                return true;
            }

            // git status / branch / tags / remotes
            if p.starts_with("git status")
                || p.starts_with("git branch")
                || p.starts_with("git tag")
                || p.starts_with("git remote")
                || p.starts_with("git show-ref")
                || p.starts_with("git log -n 1")
                || p.starts_with("git log --oneline -n")
            {
                return true;
            }

            // which / where
            if p.starts_with("which ") || p.starts_with("whereis ") || p.starts_with("type ") {
                return true;
            }

            // Markdown file reads (e.g. cat README.md, head docs/guide.md)
            if imc_proxy_copilot::context_filters::is_markdown_read(p, "") {
                return true;
            }
        }
    }
    false
}

/// Fallback content heuristic to detect search results, grep hits, and directory listings.
pub fn is_search_or_discovery_content(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lines: Vec<&str> = trimmed
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    if lines.is_empty() {
        return false;
    }

    // Git status markers
    if lines.iter().any(|l| {
        l.starts_with("On branch ")
            || l.starts_with("Changes to be committed:")
            || l.starts_with("Untracked files:")
            || l.starts_with("Changes not staged for commit:")
            || l.starts_with("nothing to commit")
    }) {
        return true;
    }

    // IMC search indicators
    if lines.iter().any(|l| {
        l.contains("more matches exist — narrow the query")
            || l.starts_with("No results found.")
            || l.starts_with("no matches for ")
    }) {
        return true;
    }

    // Numbered search hits: "1. [chunk] ... (score 0.xxxx)" or "1. [struct] ..."
    let numbered_hits = lines
        .iter()
        .filter(|l| {
            let is_numbered = l
                .split_once('.')
                .map(|(num, rest)| {
                    num.chars().all(|c| c.is_ascii_digit())
                        && !num.is_empty()
                        && rest.starts_with(" [")
                })
                .unwrap_or(false);
            let has_score = l.contains("(score ") || l.contains("[score ");
            is_numbered || has_score
        })
        .count();

    if numbered_hits >= 2 || (lines.len() <= 5 && numbered_hits >= 1) {
        return true;
    }

    // Grep / rg lines: "path/to/file:123:content"
    let grep_lines = lines
        .iter()
        .filter(|l| {
            if let Some((path, rest)) = l.split_once(':') {
                if !path.is_empty() && !path.contains(' ') {
                    if let Some((num_str, _)) = rest.split_once(':') {
                        return num_str.chars().all(|c| c.is_ascii_digit()) && !num_str.is_empty();
                    }
                }
            }
            false
        })
        .count();

    if grep_lines >= 3 && grep_lines * 2 >= lines.len() {
        return true;
    }

    // Directory listing format: "-rw-r--r--" or "drwxr-xr-x"
    let ls_lines = lines
        .iter()
        .filter(|l| {
            (l.starts_with('-') || l.starts_with('d') || l.starts_with('l'))
                && l.len() >= 10
                && l[1..10].chars().all(|c| matches!(c, 'r' | 'w' | 'x' | '-'))
        })
        .count();

    if ls_lines >= 3 && ls_lines * 2 >= lines.len() {
        return true;
    }

    false
}

/// Decide if this tool result should be bypassed from compression.
pub fn should_bypass_compression(
    tool_meta: Option<&(String, Option<String>)>,
    text: &str,
) -> bool {
    if let Some((name, cmd_opt)) = tool_meta {
        let name_lower = name.trim().to_lowercase();

        // 1. Tool name indicates search, discovery, navigation, or retrieval
        if is_bypass_tool_name(&name_lower) {
            return true;
        }

        // 2. Reading a markdown file (.md, .markdown, .mdx, etc.) — do not compress
        if let Some(cmd) = cmd_opt {
            if imc_proxy_copilot::context_filters::is_markdown_read(cmd, &name_lower) {
                return true;
            }
        }

        // 3. Shell / command execution tool: inspect command line
        if is_exec_tool_name(&name_lower) {
            if let Some(cmd) = cmd_opt {
                if is_bypass_command(cmd) {
                    return true;
                }
            }
        }
    }

    // 4. Fallback content heuristic: markdown content, search results, or status
    if imc_proxy_copilot::context_filters::is_markdown_content(text) {
        return true;
    }

    is_search_or_discovery_content(text)
}

/// Walk every tool result in the conversation and compress the big ones.
pub fn compress_tool_results(
    messages: &mut [AnthropicInputMessage],
    db: &Path,
) -> CompressionReport {
    let mut report = CompressionReport::default();
    let Ok(mut store) = CcrStore::open(db) else {
        // No store means nowhere to put the original: forward untouched rather
        // than compress irreversibly.
        return report;
    };

    // Identify tool_use_ids corresponding to retrieve operations and index tool uses
    let mut retrieve_tool_ids = std::collections::HashSet::new();
    let mut tool_uses: HashMap<String, (String, Option<String>)> = HashMap::new();
    for msg in messages.iter() {
        if let AnthropicInputMessage::Assistant(assistant) = msg {
            if let StringOrBlocks::Blocks(blocks) = &assistant.content {
                for b in blocks {
                    if let AnthropicAssistantContentBlock::ToolUse(tu) = b {
                        if tu.name.contains("retrieve") {
                            retrieve_tool_ids.insert(tu.id.clone());
                        } else if let Some(cmd) = tu.input.get("command").and_then(|v| v.as_str()) {
                            if cmd.contains("retrieve") {
                                retrieve_tool_ids.insert(tu.id.clone());
                            }
                        }
                        let cmd_or_query = extract_command_or_query(&tu.input);
                        tool_uses.insert(
                            tu.id.clone(),
                            (tu.name.clone(), cmd_or_query),
                        );
                    }
                }
            }
        }
    }

    for message in messages.iter_mut() {
        let AnthropicInputMessage::User(user) = message else {
            continue;
        };
        let StringOrBlocks::Blocks(blocks) = &mut user.content else {
            continue;
        };
        for block in blocks.iter_mut() {
            let AnthropicUserContentBlock::ToolResult(result) = block else {
                continue;
            };
            // The error text *is* the answer.
            if result.is_error == Some(true) {
                continue;
            }
            // An explicit retrieve operation means the caller wanted the full original text.
            if retrieve_tool_ids.contains(&result.tool_use_id) {
                continue;
            }

            let tool_meta = tool_uses.get(&result.tool_use_id);
            match &mut result.content {
                StringOrBlocks::Text(text) => {
                    if !should_bypass_compression(tool_meta, text) {
                        maybe_shrink(text, &mut store, &mut report);
                    }
                }
                StringOrBlocks::Blocks(inner) => {
                    for b in inner.iter_mut() {
                        if let AnthropicToolResultContentBlock::Text(t) = b {
                            if !should_bypass_compression(tool_meta, &t.text) {
                                maybe_shrink(&mut t.text, &mut store, &mut report);
                            }
                        }
                    }
                }
            }
        }
    }
    report
}

/// Compress one text payload in place if it is big enough and the win is real.
fn maybe_shrink(text: &mut String, store: &mut CcrStore, report: &mut CompressionReport) {
    report.considered += 1;
    if text.len() < MIN_TOKENS * 4 {
        return;
    }
    // Never re-compress content that is already compressed or already carries an imc CCR handle.
    if (text.contains("[imc: ") && text.contains("full output is id "))
        || text.contains("## Shortened (full text on demand)")
    {
        return;
    }

    let result = compress(text);
    if result.original_tokens == 0 {
        return;
    }
    let saved = result.saved_tokens as f64 / result.original_tokens as f64;
    // Read min_savings from config, fall back to the compile-time default.
    let min_savings = settings::load()
        .compression
        .as_ref()
        .and_then(|c| c.min_savings)
        .unwrap_or(MIN_SAVINGS);
    if saved < min_savings {
        return;
    }

    let id = match store.store(
        kind_slug(result.kind),
        text,
        &result.compressed,
        result.original_tokens as i64,
        result.compressed_tokens as i64,
    ) {
        Ok(id) => id,
        Err(_) => return,
    };

    report.compressed += 1;
    report.original_tokens += result.original_tokens;
    report.compressed_tokens += result.compressed_tokens;
    *text = format!(
        "{}\n\n[imc: {} -> {} tokens; full output is id {}. Retrieve with `imc retrieve {}` or POST /v1/retrieve {{\"id\":\"{}\"}}]",
        result.compressed, result.original_tokens, result.compressed_tokens, id, id, id
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use imc_proxy_types::types::anthropic::{
        AnthropicTextBlock, AnthropicToolResultBlock,
    };
    use serde_json::json;

    fn tmp_db(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("imc-ctx-{tag}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn user_with_tool_result(block: AnthropicToolResultBlock) -> AnthropicInputMessage {
        AnthropicInputMessage::User(imc_proxy_types::types::anthropic::AnthropicUserMessage {
            role: "user".to_string(),
            content: StringOrBlocks::Blocks(vec![AnthropicUserContentBlock::ToolResult(block)]),
        })
    }

    /// Pull the tool-result text out of a one-message conversation.
    fn only_tool_result_text(msgs: &[AnthropicInputMessage]) -> String {
        let AnthropicInputMessage::User(u) = &msgs[0] else {
            panic!("expected a user message")
        };
        let StringOrBlocks::Blocks(b) = &u.content else {
            panic!("expected blocks")
        };
        let AnthropicUserContentBlock::ToolResult(tr) = &b[0] else {
            panic!("expected a tool result")
        };
        match &tr.content {
            StringOrBlocks::Text(s) => s.clone(),
            StringOrBlocks::Blocks(_) => panic!("expected bare text content"),
        }
    }

    fn tool_result(content: StringOrBlocks<AnthropicToolResultContentBlock>) -> AnthropicToolResultBlock {
        AnthropicToolResultBlock {
            r#type: "tool_result".to_string(),
            tool_use_id: "toolu_1".to_string(),
            content,
            is_error: None,
            cache_control: None,
        }
    }

    #[test]
    fn small_tool_result_is_untouched() {
        let db = tmp_db("small");
        let original = "ok".to_string();
        let mut msgs = vec![user_with_tool_result(tool_result(StringOrBlocks::Text(
            original.clone(),
        )))];
        let report = compress_tool_results(&mut msgs, &db);
        assert_eq!(report.compressed, 0);
        assert_eq!(report.considered, 1);
        assert_eq!(only_tool_result_text(&msgs), original);
    }

    #[test]
    fn error_result_is_never_compressed() {
        let db = tmp_db("error");
        let big = "ERROR: could not connect\n".repeat(500);
        let mut tr = tool_result(StringOrBlocks::Text(big.clone()));
        tr.is_error = Some(true);
        let mut msgs = vec![user_with_tool_result(tr)];
        let report = compress_tool_results(&mut msgs, &db);
        assert_eq!(report.compressed, 0, "error output must survive verbatim");
    }

    #[test]
    fn big_repetitive_result_is_compressed_and_retrievable() {
        let db = tmp_db("big");
        // A tool result of the shape a build log or repeated fixture takes.
        let big = "2026-10-19 10:00:00 INFO step complete\n".repeat(300);
        let mut msgs = vec![user_with_tool_result(tool_result(StringOrBlocks::Text(
            big.clone(),
        )))];
        let report = compress_tool_results(&mut msgs, &db);
        assert_eq!(report.compressed, 1, "expected the big result to compress");
        assert!(report.saved_tokens() > 0);

        let new_text = only_tool_result_text(&msgs);
        assert!(new_text.len() < big.len(), "compressed text should be smaller");
        assert!(new_text.contains("/v1/retrieve"), "must leave a retrieval handle");

        // The handle must actually resolve to the original bytes.
        let id_start = new_text.find(r#"{"id":""#).expect("handle must carry an id") + 7;
        let id = &new_text[id_start..id_start + new_text[id_start..].find('"').unwrap()];
        let store = CcrStore::open(&db).unwrap();
        let recovered = store.get(id).unwrap().expect("id must resolve");
        assert_eq!(recovered, big, "CCR must hold the exact original");
    }

    #[test]
    fn block_list_content_is_compressed_too() {
        let db = tmp_db("blocks");
        let big = "fn main() {\n    let x = 1;\n}\n".repeat(200);
        let inner = vec![AnthropicToolResultContentBlock::Text(AnthropicTextBlock {
            r#type: "text".to_string(),
            text: big.clone(),
            cache_control: None,
        })];
        let mut msgs = vec![user_with_tool_result(tool_result(StringOrBlocks::Blocks(
            inner,
        )))];
        let report = compress_tool_results(&mut msgs, &db);
        assert_eq!(report.compressed, 1);
    }

    #[test]
    fn assistant_messages_are_left_alone() {
        let db = tmp_db("assistant");
        let mut msgs = vec![AnthropicInputMessage::Assistant(Default::default())];
        let report = compress_tool_results(&mut msgs, &db);
        assert_eq!(report.considered, 0);
        assert_eq!(report.compressed, 0);
    }

    #[test]
    fn json_object_payload_roundtrips_through_the_report() {
        // Guard the arithmetic the log line depends on.
        let r = CompressionReport {
            considered: 1,
            compressed: 1,
            original_tokens: 1000,
            compressed_tokens: 250,
        };
        assert_eq!(r.saved_tokens(), 750);
        let _ = json!({"saved": r.saved_tokens()});
    }

    fn assistant_with_tool_use(
        id: &str,
        name: &str,
        input: BTreeMap<String, Value>,
    ) -> AnthropicInputMessage {
        AnthropicInputMessage::Assistant(imc_proxy_types::types::anthropic::AnthropicAssistantMessage {
            role: "assistant".to_string(),
            content: StringOrBlocks::Blocks(vec![AnthropicAssistantContentBlock::ToolUse(
                imc_proxy_types::types::anthropic::AnthropicToolUseBlock {
                    r#type: "tool_use".to_string(),
                    id: id.to_string(),
                    name: name.to_string(),
                    input,
                    cache_control: None,
                },
            )]),
        })
    }

    #[test]
    fn search_tool_result_is_never_compressed() {
        let db = tmp_db("search-tool");
        let search_hits = (1..=20)
            .map(|i| {
                format!(
                    "{}. [chunk] crates/imc-cli/src/main.rs:{}-{} my_symbol (score 0.8877) fn main() {{}}\n",
                    i,
                    i * 10,
                    i * 10 + 20
                )
            })
            .collect::<String>()
            .repeat(5); // Make it large (> MIN_TOKENS * 4)

        let asst = assistant_with_tool_use("toolu_search", "imc_search", BTreeMap::new());
        let mut tr = tool_result(StringOrBlocks::Text(search_hits.clone()));
        tr.tool_use_id = "toolu_search".to_string();
        let user = user_with_tool_result(tr);

        let mut msgs = vec![asst, user];
        let report = compress_tool_results(&mut msgs, &db);

        assert_eq!(report.compressed, 0, "search results must be bypassed");
        assert_eq!(report.considered, 0, "bypassed tool results should not even be considered");
    }

    #[test]
    fn bash_imc_search_is_bypassed() {
        let db = tmp_db("bash-search");
        let search_hits = (1..=20)
            .map(|i| {
                format!(
                    "{}. [chunk] crates/imc-cli/src/main.rs:{}-{} my_symbol (score 0.8877) fn main() {{}}\n",
                    i,
                    i * 10,
                    i * 10 + 20
                )
            })
            .collect::<String>()
            .repeat(5);

        let mut input = BTreeMap::new();
        input.insert("command".to_string(), Value::String("imc search auth".to_string()));
        let asst = assistant_with_tool_use("toolu_bash", "bash", input);
        let mut tr = tool_result(StringOrBlocks::Text(search_hits.clone()));
        tr.tool_use_id = "toolu_bash".to_string();
        let user = user_with_tool_result(tr);

        let mut msgs = vec![asst, user];
        let report = compress_tool_results(&mut msgs, &db);

        assert_eq!(report.compressed, 0, "bash running imc search must be bypassed");
    }

    #[test]
    fn bash_grep_is_bypassed() {
        let db = tmp_db("bash-grep");
        let grep_output = (1..=50)
            .map(|i| format!("crates/imc-proxy/src/main.rs:{}:    let val = {};\n", i, i))
            .collect::<String>()
            .repeat(10);

        let mut input = BTreeMap::new();
        input.insert("command".to_string(), Value::String("grep -rn 'val' crates/".to_string()));
        let asst = assistant_with_tool_use("toolu_grep", "Bash", input);
        let mut tr = tool_result(StringOrBlocks::Text(grep_output));
        tr.tool_use_id = "toolu_grep".to_string();
        let user = user_with_tool_result(tr);

        let mut msgs = vec![asst, user];
        let report = compress_tool_results(&mut msgs, &db);

        assert_eq!(report.compressed, 0, "grep command must be bypassed");
    }

    #[test]
    fn bash_git_status_is_bypassed() {
        let db = tmp_db("bash-git-status");
        let status_output = format!(
            "On branch master\nYour branch is up to date with 'origin/master'.\n\n\
             Changes not staged for commit:\n  (use \"git add <file>...\" to update what will be committed)\n\
             {}\n\nno changes added to commit (use \"git add\" to track)\n",
            (1..=100)
                .map(|i| format!("        modified:   file_{}.rs\n", i))
                .collect::<String>()
        );

        let mut input = BTreeMap::new();
        input.insert("command".to_string(), Value::String("git status".to_string()));
        let asst = assistant_with_tool_use("toolu_status", "bash", input);
        let mut tr = tool_result(StringOrBlocks::Text(status_output));
        tr.tool_use_id = "toolu_status".to_string();
        let user = user_with_tool_result(tr);

        let mut msgs = vec![asst, user];
        let report = compress_tool_results(&mut msgs, &db);

        assert_eq!(report.compressed, 0, "git status must be bypassed");
    }

    #[test]
    fn content_heuristic_detects_search_hits_without_tool_use() {
        let db = tmp_db("heuristic-search");
        let search_hits = (1..=20)
            .map(|i| {
                format!(
                    "{}. [chunk] crates/imc-cli/src/main.rs:{}-{} my_symbol (score 0.8877) fn main() {{}}\n",
                    i,
                    i * 10,
                    i * 10 + 20
                )
            })
            .collect::<String>()
            .repeat(5);

        // Tool result has unknown tool_use_id (no preceding tool use block)
        let tr = tool_result(StringOrBlocks::Text(search_hits));
        let user = user_with_tool_result(tr);

        let mut msgs = vec![user];
        let report = compress_tool_results(&mut msgs, &db);

        assert_eq!(report.compressed, 0, "fallback heuristic must protect search hits");
    }

    #[test]
    fn build_logs_under_bash_are_still_compressed() {
        let db = tmp_db("bash-build");
        let build_log = "2026-10-19 10:00:00 INFO step complete\n".repeat(300);

        let mut input = BTreeMap::new();
        input.insert("command".to_string(), Value::String("cargo build".to_string()));
        let asst = assistant_with_tool_use("toolu_build", "bash", input);
        let mut tr = tool_result(StringOrBlocks::Text(build_log));
        tr.tool_use_id = "toolu_build".to_string();
        let user = user_with_tool_result(tr);

        let mut msgs = vec![asst, user];
        let report = compress_tool_results(&mut msgs, &db);

        assert_eq!(report.compressed, 1, "cargo build log must still be compressed");
    }

    #[test]
    fn markdown_file_reads_bypass_compression() {
        let db = tmp_db("md-bypass");
        let md_content = "# Guide\n\nIntro.\n\n## Section\n- Item\n".repeat(100);
        assert!(md_content.len() > MIN_TOKENS * 4);

        // 1. Tool use with command = "cat README.md"
        let mut input1 = BTreeMap::new();
        input1.insert("command".to_string(), Value::String("cat README.md".to_string()));
        let asst1 = assistant_with_tool_use("toolu_cat_md", "bash", input1);
        let mut tr1 = tool_result(StringOrBlocks::Text(md_content.clone()));
        tr1.tool_use_id = "toolu_cat_md".to_string();
        let user1 = user_with_tool_result(tr1);

        let mut msgs = vec![asst1, user1];
        let report = compress_tool_results(&mut msgs, &db);
        assert_eq!(report.compressed, 0, "reading README.md via cat must bypass compression");

        // 2. Tool use with view_file path = "docs/SKILL.md"
        let mut input2 = BTreeMap::new();
        input2.insert("path".to_string(), Value::String("docs/SKILL.md".to_string()));
        let asst2 = assistant_with_tool_use("toolu_view_md", "view_file", input2);
        let mut tr2 = tool_result(StringOrBlocks::Text(md_content.clone()));
        tr2.tool_use_id = "toolu_view_md".to_string();
        let user2 = user_with_tool_result(tr2);

        let mut msgs2 = vec![asst2, user2];
        let report2 = compress_tool_results(&mut msgs2, &db);
        assert_eq!(report2.compressed, 0, "view_file on SKILL.md must bypass compression");
    }
}
