//! Faithful Rust port of `main/cli/commands/find.py` (the token-optimised file
//! reader / `imc-read` command).
//!
//! Public entry points return `String` / `Result<String>` so callers (CLI,
//! hooks, tests) decide how to render — nothing prints internally.
//!
//! Core pipeline (fully ported, unit-tested):
//!   run_find -> _process_text | _process_json -> _process_claude_payload
//!   blank-line collapse, offset/limit windows, MAX_CHARS_FULL / MAX_CHARS_JSON
//!   truncation, JSON smart-strip (system-reminder / hook injections /
//!   duplicate tool_results).
//!
//! The tree-sitter powered `--symbols` outline and `--method` extraction are
//! routed through `imc-parser` (which wires python/js/ts/tsx/rust/go grammars).
//! This is a subset of the Python `tree_sitter_language_pack` languages; see
//! the module docs for the exact gaps.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{anyhow, Context, Result};
use regex::Regex;
use serde_json::{Map, Value};

// ─── Thresholds (mirrors find.py) ────────────────────────────────────────────
/// Above this char count → truncate + warn (generic text/code).
pub const MAX_CHARS_FULL: usize = 60_000;
/// JSON gets a bigger budget (structure helps).
pub const MAX_CHARS_JSON: usize = 80_000;
pub const BLANK_LINE_STRIP: bool = true;
/// Default per-tool_result truncation budget.
const MAX_TOOL_RESULT_CHARS: usize = 8_000;

// ─── Public entry point ───────────────────────────────────────────────────────

/// Options mirroring `run_find`'s keyword arguments.
#[derive(Debug, Default, Clone, Copy)]
pub struct FindOptions {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub raw: bool,
    pub method: Option<&'static str>,
    pub symbols: bool,
}

/// Port of `run_find`. Returns the string the caller should print.
///
/// `Err` covers the two failure paths from the Python (`File not found`,
/// `Not a file`), which the original turned into `sys.exit(1)` + stderr.
pub fn run_find(file_path: &str, opts: FindOptions) -> Result<String> {
    let path = resolve_path(file_path);

    if !path.exists() {
        return Err(anyhow!("[imc find] File not found: {}", path.display()));
    }
    if !path.is_file() {
        return Err(anyhow!("[imc find] Not a file: {}", path.display()));
    }

    let raw_bytes = std::fs::read(&path)
        .with_context(|| format!("[imc find] Failed to read: {}", path.display()))?;

    // ── Symbols-only mode: outline and exit ──────────────────────────────────
    if opts.symbols {
        let text = String::from_utf8_lossy(&raw_bytes);
        let outline = outline_symbols(&text, &path);
        return Ok(if outline.is_empty() {
            format!(
                "No symbols found (unsupported language or empty file): {}",
                path.display()
            )
        } else {
            outline
        });
    }

    // ── Method mode: extract the named function/method via tree-sitter ───────
    if let Some(method) = opts.method {
        let text = String::from_utf8_lossy(&raw_bytes);
        return Ok(extract_method(&text, &path, method));
    }

    if opts.raw {
        // If tree-sitter supports this file type, block --raw and show symbols.
        let text = String::from_utf8_lossy(&raw_bytes);
        let outline = outline_symbols(&text, &path);
        if !outline.is_empty() {
            let suffix = path
                .extension()
                .map(|_| {
                    let mut s = String::new();
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if let Some(idx) = name.rfind('.') {
                        s.push_str(&name[idx..]);
                    }
                    s
                })
                .unwrap_or_default();
            return Ok(format!(
                "⚠️  --raw blocked for '{suffix}' files (tree-sitter supported).\n\
                 Use --symbols to explore, then --method <name> to read a specific function.\n\
                 {}\n{}",
                path.display(),
                outline
            ));
        }
        // Unsupported language (e.g. .md, .yaml) — fall through to raw dump.
        let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        let lines = apply_offset_limit(lines, opts.offset, opts.limit);
        let start = opts.offset.unwrap_or(0);
        let out: Vec<String> = lines
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{}\t{}", start + i + 1, l))
            .collect();
        return Ok(out.join("\n"));
    }

    // Detect file type.
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if ext == "json" {
        Ok(process_json(&raw_bytes, &path, opts.limit, opts.offset))
    } else {
        Ok(process_text(&raw_bytes, &path, opts.limit, opts.offset))
    }
}

// ─── Text processor ──────────────────────────────────────────────────────────

/// Port of `_process_text`: decode, window, collapse blank runs, `cat -n`
/// numbering, then hard-truncate at [`MAX_CHARS_FULL`].
pub fn process_text(raw_bytes: &[u8], path: &Path, limit: Option<usize>, offset: Option<usize>) -> String {
    let content = String::from_utf8_lossy(raw_bytes);
    let all_lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let total_lines = all_lines.len();

    let lines = apply_offset_limit(all_lines, offset, limit);

    let lines = if BLANK_LINE_STRIP {
        // Collapse consecutive blank lines to 1.
        let mut compressed: Vec<String> = Vec::with_capacity(lines.len());
        let mut prev_blank = false;
        for line in lines {
            let is_blank = line.trim().is_empty();
            if is_blank && prev_blank {
                continue;
            }
            compressed.push(line);
            prev_blank = is_blank;
        }
        compressed
    } else {
        lines
    };

    // Re-number with cat -n style numbering (offset-aware).
    let start = offset.unwrap_or(0);
    let numbered: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{}\t{}", start + i + 1, line))
        .collect();
    let mut result = numbered.join("\n");

    // Hard truncate if still massive (by code points, as in Python).
    if result.chars().count() > MAX_CHARS_FULL {
        let truncated: String = result.chars().take(MAX_CHARS_FULL).collect();
        result = truncated;
        result.push_str(&format!(
            "\n\n[imc find] Truncated at {} chars. File has {} lines total.",
            commas(MAX_CHARS_FULL),
            commas(total_lines)
        ));
    }

    let mut header = format!(
        "[imc find] {} ({} lines",
        path.display(),
        commas(total_lines)
    );
    if offset.is_some() || limit.is_some() {
        let lim = match limit {
            Some(l) if l != 0 => commas(l),
            _ => "all".to_string(),
        };
        header.push_str(&format!(
            ", showing offset={} limit={}",
            offset.unwrap_or(0),
            lim
        ));
    }
    header.push(')');

    format!("{}\n{}", header, result)
}

// ─── JSON processor ──────────────────────────────────────────────────────────

/// Port of `_process_json`. Invalid JSON falls back to [`process_text`].
pub fn process_json(raw_bytes: &[u8], path: &Path, limit: Option<usize>, offset: Option<usize>) -> String {
    let raw_str = String::from_utf8_lossy(raw_bytes);
    let file_size = raw_bytes.len();

    let data: Value = match serde_json::from_str(&raw_str) {
        Ok(v) => v,
        // Not valid JSON → treat as text.
        Err(_) => return process_text(raw_bytes, path, limit, offset),
    };

    // Detect Claude Code API payload (has model + messages array).
    if let Some(obj) = data.as_object() {
        if obj.contains_key("messages") && obj.contains_key("model") {
            return process_claude_payload(&data, path, file_size);
        }
    }

    // Generic JSON → compact dump (separators "," / ":").
    let mut compact = serde_json::to_string(&data).unwrap_or_else(|_| raw_str.to_string());
    if compact.chars().count() > MAX_CHARS_JSON {
        let truncated: String = compact.chars().take(MAX_CHARS_JSON).collect();
        compact = format!(
            "{}\n[imc find] Truncated at {} chars",
            truncated,
            commas(MAX_CHARS_JSON)
        );
    }

    format!(
        "[imc find] {} ({} bytes, JSON compact)\n{}",
        path.display(),
        commas(file_size),
        compact
    )
}

/// Stats tracked while compressing a Claude Code payload.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct PayloadStats {
    original_msgs: usize,
    stripped_system: usize,
    stripped_blocks: usize,
    deduped_tool_results: usize,
}

/// Port of `_process_claude_payload`: smart-compress an API payload, stripping
/// injected system messages, system-reminder blocks and duplicate tool_results.
pub fn process_claude_payload(data: &Value, path: &Path, file_size: usize) -> String {
    let model = data
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let messages = data
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut compressed_msgs: Vec<Value> = Vec::new();
    let mut stats = PayloadStats {
        original_msgs: messages.len(),
        ..Default::default()
    };

    for msg in &messages {
        let role = msg.get("role").and_then(Value::as_str).unwrap_or("").to_string();
        let content = msg.get("content").cloned().unwrap_or(Value::String(String::new()));

        // ── system messages (hook injections) ────────────────────────────────
        if role == "system" {
            match compress_system_msg(&content) {
                Some(c) => compressed_msgs.push(object(vec![("role", sv("system")), ("content", c)])),
                None => stats.stripped_system += 1,
            }
            continue;
        }

        // ── string content ──────────────────────────────────────────────────
        if content.is_string() {
            compressed_msgs.push(object(vec![
                ("role", Value::String(role)),
                ("content", content),
            ]));
            continue;
        }

        // ── array content ───────────────────────────────────────────────────
        if let Some(blocks) = content.as_array() {
            let mut clean_blocks: Vec<Value> = Vec::new();
            // tool_use_id → content hash (order preserved for _find_original_id)
            let mut seen_tool_results: HashMap<String, u64> = HashMap::new();

            for block in blocks {
                let btype = block.get("type").and_then(Value::as_str).unwrap_or("");

                match btype {
                    // text blocks — strip injected XML-like blocks
                    "text" => {
                        let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                        let cleaned = strip_system_injections(text);
                        if !cleaned.trim().is_empty() {
                            clean_blocks.push(object(vec![
                                ("type", sv("text")),
                                ("text", Value::String(cleaned)),
                            ]));
                        } else {
                            stats.stripped_blocks += 1;
                        }
                    }
                    // tool_use — keep but compact input
                    "tool_use" => clean_blocks.push(object(vec![
                        ("type", sv("tool_use")),
                        ("id", block.get("id").cloned().unwrap_or_else(|| sv(""))),
                        ("name", block.get("name").cloned().unwrap_or_else(|| sv(""))),
                        ("input", block.get("input").cloned().unwrap_or_else(|| Value::Object(Map::new()))),
                    ])),
                    // tool_result — deduplicate + truncate
                    "tool_result" => {
                        let tid = block
                            .get("tool_use_id")
                            .cloned()
                            .unwrap_or(Value::String(String::new()));
                        let result_content = block
                            .get("content")
                            .cloned()
                            .unwrap_or(Value::String(String::new()));
                        let result_str = value_display_str(&result_content);
                        let content_hash = stable_hash(&result_str);

                        if seen_tool_results.values().any(|&h| h == content_hash) {
                            stats.deduped_tool_results += 1;
                            let original = find_original_id(&seen_tool_results, content_hash);
                            clean_blocks.push(object(vec![
                                ("type", sv("tool_result")),
                                ("tool_use_id", tid),
                                (
                                    "content",
                                    Value::String(format!(
                                        "[imc find] DUPLICATE — same as tool_result id={original}"
                                    )),
                                ),
                            ]));
                        } else {
                            let tid_key = tid.as_str().unwrap_or("").to_string();
                            seen_tool_results.insert(tid_key, content_hash);
                            let truncated = truncate_tool_result(result_content, MAX_TOOL_RESULT_CHARS);
                            let mut fields = vec![
                                ("type", Value::String("tool_result".into())),
                                ("tool_use_id", tid),
                                ("content", truncated),
                            ];
                            if let Some(is_err) = block.get("is_error") {
                                fields.push(("is_error", is_err.clone()));
                            }
                            clean_blocks.push(object(fields));
                        }
                    }
                    // unknown block kinds pass through untouched
                    _ => clean_blocks.push(block.clone()),
                }
            }

            if !clean_blocks.is_empty() {
                compressed_msgs.push(object(vec![
                    ("role", Value::String(role)),
                    ("content", Value::Array(clean_blocks)),
                ]));
            }
        }
        // Any other content type (None, number, object) is dropped, as in Python.
    }

    let mut result_obj = Map::new();
    result_obj.insert("model".into(), Value::String(model));
    result_obj.insert("messages".into(), Value::Array(compressed_msgs.clone()));
    let result_data = Value::Object(result_obj);

    // Pretty-printed with indent 2, ensure_ascii=False (Rust keeps unicode).
    let output_str = serde_json::to_string_pretty(&result_data).unwrap_or_default();

    let orig_size = file_size;
    let new_size = output_str.as_bytes().len();
    let savings_str = if orig_size > 0 {
        format!("{:.1}", (1.0 - new_size as f64 / orig_size as f64) * 100.0)
    } else {
        "0".to_string()
    };

    let header = format!(
        "[imc find] {} — Claude Code payload\n  Original: {} bytes | Compressed: {} bytes | Saved: {}%\n  Messages: {} → {} (stripped {} system, {} injected blocks, {} duplicate tool_results)\n",
        path.display(),
        commas(orig_size),
        commas(new_size),
        savings_str,
        stats.original_msgs,
        compressed_msgs.len(),
        stats.stripped_system,
        stats.stripped_blocks,
        stats.deduped_tool_results
    );

    format!("{}{}", header, output_str)
}

// ─── Payload helpers ─────────────────────────────────────────────────────────

/// Port of `_compress_system_msg`. `None` means "drop this system message".
fn compress_system_msg(content: &Value) -> Option<Value> {
    // Non-string system content passes through unchanged.
    let Some(text) = content.as_str() else {
        return Some(content.clone());
    };

    // IDE file-open notification → keep (short, contextual).
    if text.starts_with("The user opened the file") {
        return Some(Value::String(text.split('\n').next().unwrap_or("").to_string()));
    }

    // Hook success notifications with full rules → summarise.
    if text.contains("CAVEMAN MODE ACTIVE") {
        return Some(Value::String("[hook] Caveman mode active (full)".into()));
    }

    if text.contains("UserPromptSubmit hook") && text.contains("imc search") {
        return Some(Value::String("[hook] imc search mandatory".into()));
    }

    // SessionStart with huge agent/skill dump → strip.
    if text.contains("Available agent types")
        || text.contains("The following skills are available")
    {
        let first_line = text.trim().split('\n').next().unwrap_or("");
        let head: String = first_line.chars().take(120).collect();
        return Some(Value::String(format!("[hook] {head}")));
    }

    // Short system messages → keep as-is.
    if text.chars().count() < 300 {
        return Some(Value::String(text.to_string()));
    }

    // Drop large unrecognised system messages.
    None
}

fn injection_regexes() -> &'static Vec<Regex> {
    static RES: OnceLock<Vec<Regex>> = OnceLock::new();
    RES.get_or_init(|| {
        [
            "system-reminder",
            "local-command-caveat",
            "command-name",
            "command-message",
            "command-args",
            "local-command-stdout",
        ]
        .iter()
        .map(|tag| Regex::new(&format!(r"(?s)<{0}>.*?</{0}>", tag)).expect("valid injection regex"))
        .collect()
    })
}

/// Port of `_strip_system_injections`: remove injected XML-like blocks.
pub fn strip_system_injections(text: &str) -> String {
    let mut out = text.to_string();
    for re in injection_regexes() {
        out = re.replace_all(&out, "").into_owned();
    }
    out.trim().to_string()
}

/// Port of `_truncate_tool_result`. Short values keep their original type.
fn truncate_tool_result(content: Value, max_chars: usize) -> Value {
    let s = value_display_str(&content);
    if s.chars().count() <= max_chars {
        return content;
    }
    let head: String = s.chars().take(max_chars).collect();
    Value::String(format!(
        "{}\n[imc find] ... truncated ({} → {} chars)",
        head,
        commas(s.chars().count()),
        commas(max_chars)
    ))
}

/// Approximate of Python's `str(value)` for hashing/truncation: plain strings
/// without quotes, other values as compact JSON.
fn value_display_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Port of `_find_original_id`.
fn find_original_id(seen: &HashMap<String, u64>, content_hash: u64) -> String {
    for (tid, h) in seen.iter() {
        if *h == content_hash {
            return tid.clone();
        }
    }
    "?".to_string()
}

fn stable_hash(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    s.hash(&mut h);
    h.finish()
}

// ─── Shared helpers ──────────────────────────────────────────────────────────

/// Build a JSON object from `(key, value)` pairs, preserving insertion order.
fn object(fields: Vec<(&str, Value)>) -> Value {
    let mut map = Map::new();
    for (k, v) in fields {
        map.insert(k.to_string(), v);
    }
    Value::Object(map)
}

/// Convenience: borrow a `&str` as a JSON string value.
fn sv(s: &str) -> Value {
    Value::String(s.to_string())
}

/// Format an integer with thousands separators, like Python's `{:,}`.
fn commas(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Port of `_apply_offset_limit`. Note: Python treats `offset == 0` as falsy,
/// so `Some(0)` behaves like `None` — preserved here for fidelity.
pub fn apply_offset_limit(lines: Vec<String>, offset: Option<usize>, limit: Option<usize>) -> Vec<String> {
    let mut lines = lines;
    if let Some(o) = offset {
        if o != 0 {
            lines = if o >= lines.len() {
                Vec::new()
            } else {
                lines.split_off(o)
            };
        }
    }
    if let Some(l) = limit {
        if l != 0 && lines.len() > l {
            lines.truncate(l);
        }
    }
    lines
}

/// Expand a leading `~` and make the path absolute without requiring existence
/// (mirrors `Path.expanduser().resolve()` with `strict=False`).
fn resolve_path(file_path: &str) -> PathBuf {
    let mut p = PathBuf::from(file_path);
    if let Ok(rest) = p.strip_prefix("~") {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            let mut full = PathBuf::from(home);
            full.push(rest);
            p = full;
        }
    }
    if p.is_absolute() {
        p
    } else if let Ok(cwd) = std::env::current_dir() {
        cwd.join(p)
    } else {
        p
    }
}

// ─── Symbols outline & method extraction (tree-sitter via imc-parser) ─────────

/// Map a parser symbol kind onto the outline's compact kind label.
fn outline_kind(kind: &str) -> &'static str {
    match kind {
        "function" | "method" | "component" => "def",
        "enum" => "enum",
        "interface" | "type" => "type",
        _ => "class",
    }
}

/// Port of `_outline_symbols` — compact `Lstart-end  kind  name` listing.
///
/// Backed by `imc-parser`'s tree-sitter extraction (python/js/ts/tsx/rust/go).
/// Returns an empty string for unsupported languages or empty files.
pub fn outline_symbols(text: &str, path: &Path) -> String {
    let Some(parsed) = imc_parser::parse(&path.to_string_lossy(), text) else {
        return String::new();
    };
    if parsed.symbols.is_empty() {
        return String::new();
    }

    // Stable, source-ordered, deduped on (start_line, name) like the Python.
    let mut syms = parsed.symbols.clone();
    syms.sort_by(|a, b| a.line.cmp(&b.line).then(a.start_byte.cmp(&b.start_byte)));

    let mut seen = std::collections::HashSet::new();
    let mut deduped: Vec<(usize, usize, &'static str, String)> = Vec::new();
    for s in &syms {
        if s.name.is_empty() {
            continue;
        }
        let key = (s.line, s.name.clone());
        if seen.insert(key) {
            deduped.push((s.line, s.end_line, outline_kind(s.kind.as_str()), s.name.clone()));
        }
    }
    if deduped.is_empty() {
        return String::new();
    }

    let width = deduped.last().unwrap().0.to_string().len();
    let lines: Vec<String> = deduped
        .iter()
        .map(|(sl, el, kind, name)| {
            format!("  L{sl:>width$}-{el:<width$}  {kind:5}  {name}", width = width)
        })
        .collect();
    lines.join("\n")
}

/// Port of `_extract_method` — extract a single function/method/class by name,
/// returning its source with `cat -n` style line numbers.
pub fn extract_method(text: &str, path: &Path, method: &str) -> String {
    let rel = path.to_string_lossy().to_string();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_lowercase()))
        .unwrap_or_default();

    // Normalise target: `foo()` / `obj.foo` → `foo`.
    let target_method = method
        .trim_end_matches("()")
        .rsplit('.')
        .next()
        .unwrap_or(method)
        .trim()
        .to_string();

    // Reject extensions without a tree-sitter grammar (as in the Python).
    let supported = imc_core::ingest::language_of(&rel)
        .map(|lang| imc_parser::SUPPORTED.contains(&lang))
        .unwrap_or(false);
    if !supported {
        return format!(
            "[imc find] Method extraction unsupported for '{ext}' files. Use --raw to read the whole file."
        );
    }

    // 1. Tree-sitter AST match (via imc-parser symbols).
    let mut parts: Vec<String> = Vec::new();
    if let Some(parsed) = imc_parser::parse(&rel, text) {
        let all_lines: Vec<&str> = text.lines().collect();
        for sym in &parsed.symbols {
            if sym.name != target_method {
                continue;
            }
            let start = sym.line.saturating_sub(1);
            let end = sym.end_line.min(all_lines.len());
            if start >= end && sym.end_line == sym.line {
                continue;
            }
            let body: Vec<String> = all_lines[start..end]
                .iter()
                .enumerate()
                .map(|(i, l)| format!("{}\t{}", start + i + 1, l))
                .collect();
            let header = format!(
                "{}:{}-{} {}",
                path.display(),
                sym.line,
                sym.end_line,
                target_method
            );
            parts.push(format!("{}\n{}", header, body.join("\n")));
        }
    }

    if !parts.is_empty() {
        return parts.join("\n\n");
    }

    // 2. Fallback: scan for a declaration line (grammar ambiguity).
    if let Some(found) = fallback_decl_block(text, path, &target_method) {
        return found;
    }

    // 3. Suggest similar symbol names.
    let suggestions = suggest_symbols(text, path, &target_method, 5);
    if !suggestions.is_empty() {
        let hint: Vec<String> = suggestions
            .iter()
            .map(|s| format!("  imc-read {} --method {}", path.display(), s))
            .collect();
        return format!(
            "[imc find] Method '{method}' not found in {}.\nDid you mean one of these?\n{}",
            path.display(),
            hint.join("\n")
        );
    }

    format!(
        "[imc find] Method '{method}' not found in {}. Use --symbols to list all available symbols.",
        path.display()
    )
}

fn fallback_decl_block(text: &str, path: &Path, target: &str) -> Option<String> {
    let pattern = Regex::new(&format!(
        r"(?:(?:public|private|protected|static|final|native|synchronized|abstract|default|async|export)\s+)*(?:[\w<>\[\],\s]+\s+)?\b{}\b\s*[\(=]",
        regex::escape(target)
    ))
    .ok()?;

    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let stripped = line.trim_start();
        if stripped.starts_with("//")
            || stripped.starts_with("/*")
            || stripped.starts_with('*')
            || stripped.starts_with('#')
            || stripped.starts_with("return ")
            || stripped.starts_with("if ")
            || stripped.starts_with("while ")
            || stripped.starts_with("for ")
            || stripped.contains(&format!(".{}", target))
        {
            continue;
        }
        if pattern.is_match(line) {
            let start_l = i;
            let end_l = (start_l + 30).min(lines.len().saturating_sub(1));
            let body: Vec<String> = lines[start_l..=end_l]
                .iter()
                .enumerate()
                .map(|(j, l)| format!("{}\t{}", start_l + j + 1, l))
                .collect();
            let header = format!(
                "[imc find] {}:{} (lines {}-{})",
                path.display(),
                target,
                start_l + 1,
                end_l + 1
            );
            return Some(format!("{}\n{}", header, body.join("\n")));
        }
    }
    None
}

/// Simplified port of `_suggest_symbols` — substring/token-overlap scoring.
fn suggest_symbols(text: &str, path: &Path, query: &str, max_results: usize) -> Vec<String> {
    let rel = path.to_string_lossy().to_string();
    let Some(parsed) = imc_parser::parse(&rel, text) else {
        return Vec::new();
    };
    let q_lower = query.to_lowercase();
    let mut scored: Vec<(f64, String)> = Vec::new();
    for s in &parsed.symbols {
        if s.name.is_empty() {
            continue;
        }
        let n_lower = s.name.to_lowercase();
        let mut sc = 0.0;
        if q_lower == n_lower {
            sc += 20.0;
        } else if q_lower.contains(&n_lower) || n_lower.contains(&q_lower) {
            sc += 10.0;
        }
        if n_lower.contains(&q_lower) && q_lower.len() > 2 {
            sc += 3.0;
        }
        if sc > 0.0 {
            scored.push((sc, s.name.clone()));
        }
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (_, n) in scored {
        if seen.insert(n.clone()) {
            out.push(n);
        }
        if out.len() >= max_results {
            break;
        }
    }
    out
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // helpers -------------------------------------------------------------
    fn tmp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("imc-find-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }
    fn write_temp(name: &str, content: &str) -> PathBuf {
        let p = tmp_path(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    // blank-line stripping ------------------------------------------------
    #[test]
    fn collapses_consecutive_blank_lines_to_one() {
        let input = "a\n\n\n\n\nb\n\n\nc\n";
        let out = process_text(input.as_bytes(), Path::new("x.txt"), None, None);
        // header + numbered lines; runs of blanks collapse to a single blank.
        let body = out.lines().skip(1).collect::<Vec<_>>();
        assert_eq!(body.len(), 5, "got {body:?}");
        assert_eq!(body[0], "1\ta");
        assert_eq!(body[1], "2\t");
        assert_eq!(body[2], "3\tb");
        assert_eq!(body[3], "4\t");
        assert_eq!(body[4], "5\tc");
    }

    #[test]
    fn blank_strip_keeps_single_leading_blank() {
        let input = "\n\nx\n";
        let out = process_text(input.as_bytes(), Path::new("x.txt"), None, None);
        let body = out.lines().skip(1).collect::<Vec<_>>();
        assert_eq!(body, vec!["1\t", "2\tx"]);
    }

    // offset / limit -----------------------------------------------------
    #[test]
    fn offset_limit_windowing() {
        let input = "l1\nl2\nl3\nl4\nl5\n";
        let out = process_text(input.as_bytes(), Path::new("x.txt"), Some(2), Some(2));
        // header mentions offset/limit
        assert!(out.starts_with("[imc find] x.txt (5 lines, showing offset=2 limit=2)"), "{out}");
        let body = out.lines().skip(1).collect::<Vec<_>>();
        assert_eq!(body, vec!["3\tl3", "4\tl4"]);
    }

    #[test]
    fn offset_only_starts_numbering_at_offset() {
        let input = "l1\nl2\nl3\nl4\n";
        let out = process_text(input.as_bytes(), Path::new("x.txt"), None, Some(3));
        let body = out.lines().skip(1).collect::<Vec<_>>();
        assert_eq!(body, vec!["4\tl4"]);
    }

    #[test]
    fn offset_beyond_eof_yields_empty_body() {
        let input = "l1\nl2\n";
        let out = process_text(input.as_bytes(), Path::new("x.txt"), None, Some(50));
        assert_eq!(out, "[imc find] x.txt (2 lines, showing offset=50 limit=all)\n");
    }

    #[test]
    fn apply_offset_limit_treats_zero_like_none() {
        let lines: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let r = apply_offset_limit(lines.clone(), Some(0), Some(0));
        assert_eq!(r, lines);
    }

    // truncation at MAX_CHARS_FULL ---------------------------------------
    #[test]
    fn truncates_text_at_max_chars_full() {
        // Build a file whose numbered body exceeds MAX_CHARS_FULL.
        let mut input = String::new();
        let line = "x".repeat(100);
        for _ in 0..800 {
            input.push_str(&line);
            input.push('\n');
        }
        let out = process_text(input.as_bytes(), Path::new("big.txt"), None, None);
        assert!(out.contains("[imc find] Truncated at 60,000 chars."), "missing trunc notice");
        assert!(out.contains("File has 800 lines total."), "missing line total");
        // Body portion (excluding the trailing notice) is capped.
        let body_start = out.find('\n').unwrap() + 1;
        let body = &out[body_start..];
        let notice_idx = body.find("\n\n[imc find] Truncated").unwrap();
        assert_eq!(body[..notice_idx].chars().count(), MAX_CHARS_FULL);
    }

    #[test]
    fn under_threshold_text_is_not_truncated() {
        let input = "short\n";
        let out = process_text(input.as_bytes(), Path::new("s.txt"), None, None);
        assert!(!out.contains("Truncated"));
    }

    // JSON smart-strip ----------------------------------------------------
    fn claude_payload_value() -> Value {
        serde_json::json!({
            "model": "claude-x",
            "messages": [
                {"role": "user", "content": "hello world"},
                {"role": "assistant", "content": [
                    {"type": "text", "text": "pre<system-reminder>INJECTED</system-reminder>post"},
                    {"type": "text", "text": "<local-command-caveat>noise</local-command-caveat>"},
                    {"type": "tool_use", "id": "tu1", "name": "Read", "input": {"path": "a.txt"}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "tr1", "content": "RESULT"},
                    {"type": "tool_result", "tool_use_id": "tr2", "content": "RESULT"}
                ]},
                {"role": "system", "content": "The user opened the file /tmp/a.txt in the editor.\nsecond line"}
            ]
        })
    }

    #[test]
    fn smart_strip_removes_system_reminder_and_empty_text() {
        let out = process_claude_payload(&claude_payload_value(), Path::new("p.json"), 400);
        assert!(out.contains("Claude Code payload"));
        assert!(!out.contains("INJECTED"), "system-reminder not stripped:\n{out}");
        assert!(!out.contains("local-command-caveat"), "caveat not stripped:\n{out}");
        // The caveat-only text block collapses to empty and is dropped, leaving
        // exactly one surviving text block (the one with residual 'prepost').
        assert_eq!(out.matches("\"type\": \"text\"").count(), 1, "{out}");
        assert!(out.contains("prepost"), "{out}");
        // Stats live in the human-readable header line.
        assert!(out.contains("1 injected blocks"), "{out}");
    }

    #[test]
    fn smart_strip_dedupes_duplicate_tool_results() {
        let out = process_claude_payload(&claude_payload_value(), Path::new("p.json"), 400);
        assert!(out.contains("DUPLICATE — same as tool_result id=tr1"), "{out}");
        assert!(out.contains("1 duplicate tool_results"), "{out}");
    }

    #[test]
    fn smart_strip_collapses_system_hook_message() {
        let out = process_claude_payload(&claude_payload_value(), Path::new("p.json"), 400);
        // The system message is kept but reduced to its first line.
        assert!(out.contains("The user opened the file /tmp/a.txt in the editor."), "{out}");
        assert!(!out.contains("second line"), "{out}");
    }

    #[test]
    fn smart_strip_drops_large_unknown_system_message() {
        let payload = serde_json::json!({
            "model": "m",
            "messages": [
                {"role": "system", "content": "Z".repeat(400)},
                {"role": "user", "content": "keep"}
            ]
        });
        let out = process_claude_payload(&payload, Path::new("p.json"), 500);
        assert!(out.contains("stripped 1 system"), "{out}");
        assert!(out.contains("keep"));
    }

    #[test]
    fn generic_json_is_compact_dumped() {
        let data = serde_json::json!({"b": 1, "a": [1, 2, {"z": "y"}]});
        let out = process_json(data.to_string().as_bytes(), Path::new("g.json"), None, None);
        assert!(out.contains("JSON compact"), "{out}");
        assert!(!out.contains("JSON compact\n{\n"), "should be compact, got {out}");
        // Compact form has no spaces after colons/commas at top level.
        let body = out.splitn(2, '\n').nth(1).unwrap();
        assert!(!body.contains(", "), "not compact: {body}");
    }

    #[test]
    fn invalid_json_falls_back_to_text() {
        let out = process_json(b"{not json", Path::new("bad.json"), None, None);
        assert!(out.starts_with("[imc find] bad.json (1 lines"), "{out}");
        assert!(out.contains("1\t{not json"));
    }

    #[test]
    fn json_with_messages_and_model_routes_to_payload() {
        let payload = claude_payload_value().to_string();
        let out = process_json(payload.as_bytes(), Path::new("p.json"), None, None);
        assert!(out.contains("Claude Code payload"), "{out}");
    }

    #[test]
    fn generic_json_truncates_at_max_chars_json() {
        let mut big = String::new();
        for _ in 0..90_000 {
            big.push('a');
        }
        let data = serde_json::json!({"k": big});
        let out = process_json(data.to_string().as_bytes(), Path::new("g.json"), None, None);
        assert!(out.contains("[imc find] Truncated at 80,000 chars"), "{out}");
    }

    #[test]
    fn tool_result_truncation_marks_size() {
        let big = "z".repeat(9000);
        let payload = serde_json::json!({
            "model": "m",
            "messages": [{"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "tr", "content": big}
            ]}]
        });
        let out = process_claude_payload(&payload, Path::new("p.json"), 20_000);
        assert!(out.contains("[imc find] ... truncated (9,000 → 8,000 chars)"), "{out}");
    }

    #[test]
    fn commas_formats_thousands() {
        assert_eq!(commas(0), "0");
        assert_eq!(commas(999), "999");
        assert_eq!(commas(1000), "1,000");
        assert_eq!(commas(60_000), "60,000");
        assert_eq!(commas(1_234_567), "1,234,567");
    }

    #[test]
    fn strip_system_injections_removes_all_tags() {
        let t = "a<system-reminder>x</system-reminder>b<command-args>y</command-args>c";
        assert_eq!(strip_system_injections(t), "abc");
    }

    // missing-file error path ---------------------------------------------
    #[test]
    fn missing_file_returns_error() {
        let p = tmp_path("does_not_exist_zzz.txt");
        let _ = std::fs::remove_file(&p);
        let err = run_find(p.to_str().unwrap(), FindOptions::default()).unwrap_err();
        assert!(err.to_string().contains("File not found"), "{err}");
    }

    #[test]
    fn directory_returns_not_a_file_error() {
        let dir = tmp_path("");
        let err = run_find(dir.to_str().unwrap(), FindOptions::default()).unwrap_err();
        assert!(err.to_string().contains("Not a file"), "{err}");
    }

    // run_find integration ------------------------------------------------
    #[test]
    fn run_find_text_end_to_end() {
        let p = write_temp("rf.txt", "alpha\n\n\nbeta\n");
        let out = run_find(p.to_str().unwrap(), FindOptions::default()).unwrap();
        assert!(out.contains("(4 lines)"), "{out}");
        assert!(out.contains("2\tbeta") || out.contains("3\tbeta"), "{out}");
    }

    #[test]
    fn run_find_json_end_to_end() {
        let p = write_temp("rf.json", "{\"a\":1,\"b\":[2,3]}");
        let out = run_find(p.to_str().unwrap(), FindOptions::default()).unwrap();
        assert!(out.contains("JSON compact"), "{out}");
    }

    #[test]
    fn run_find_raw_unsupported_language_dumps_lines() {
        let p = write_temp("rf.md", "# Title\n\nbody\n");
        let out = run_find(
            p.to_str().unwrap(),
            FindOptions { raw: true, ..Default::default() },
        )
        .unwrap();
        // .md is not tree-sitter supported → raw numbered dump.
        assert!(out.starts_with("1\t# Title"), "{out}");
    }

    #[test]
    fn run_find_symbols_unsupported_reports_message() {
        let p = write_temp("rf_sym.md", "hello\n");
        let out = run_find(
            p.to_str().unwrap(),
            FindOptions { symbols: true, ..Default::default() },
        )
        .unwrap();
        assert!(out.starts_with("No symbols found"), "{out}");
    }

    #[test]
    fn run_find_symbols_lists_python_symbols() {
        let p = write_temp("rf_sym.py", "def alpha():\n    return 1\n\n\nclass Beta:\n    pass\n");
        let out = run_find(
            p.to_str().unwrap(),
            FindOptions { symbols: true, ..Default::default() },
        )
        .unwrap();
        assert!(out.contains("alpha"), "{out}");
        assert!(out.contains("Beta"), "{out}");
        assert!(out.contains("def"), "{out}");
    }

    #[test]
    fn run_find_raw_blocked_for_supported_language() {
        let p = write_temp("rf_raw.py", "def a():\n    return 1\n");
        let out = run_find(
            p.to_str().unwrap(),
            FindOptions { raw: true, ..Default::default() },
        )
        .unwrap();
        assert!(out.contains("--raw blocked"), "{out}");
    }

    #[test]
    fn extract_method_returns_numbered_block() {
        let p = tmp_path("em.py");
        let src = "def target():\n    x = 1\n    return x\n";
        std::fs::write(&p, src).unwrap();
        let out = extract_method(src, &p, "target");
        assert!(out.contains("target"), "{out}");
        assert!(out.contains("1\tdef target():"), "{out}");
        assert!(out.contains("3\t    return x"), "{out}");
    }

    #[test]
    fn extract_method_missing_reports_not_found() {
        let p = tmp_path("em2.py");
        let src = "def alpha():\n    return 1\n";
        std::fs::write(&p, src).unwrap();
        let out = extract_method(src, &p, "zzzz_missing");
        assert!(out.contains("not found"), "{out}");
    }
}
