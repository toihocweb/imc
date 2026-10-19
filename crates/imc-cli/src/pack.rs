//! `imc pack` — assemble the context an agent actually needs, under a budget.
//!
//! Every other command answers one question: `search` finds chunks, `trace`
//! walks the graph, `memory-search` recalls rules, `compress` shrinks text.
//! An agent asking "how does authentication work here?" has to run all four and
//! then decide what fits in its window. `pack` does that assembly once:
//!
//!   1. hybrid retrieval for the query,
//!   2. graph neighbourhood for the top symbols (what they call, who calls
//!      them) so the answer isn't a naked snippet,
//!   3. project memories that bear on the query,
//!   4. compression to fit a token budget, with the full text parked in the CCR
//!      store and a handle emitted next to each item.
//!
//! The result is self-contained: paths, line ranges, symbol names, provenance
//! (`why this is here`), and a handle for anything that was shortened.

use anyhow::Result;
use imc_core::store::Store;
use imc_core::types::SearchHit;

/// How a token budget is divided. Retrieval gets the bulk; memories and graph
/// facts are cheap but high-value, so they are reserved up front rather than
/// competing with code for the same space.
const MEMORY_SHARE: f64 = 0.15;
const GRAPH_SHARE: f64 = 0.10;

/// A packed item, ready to render.
#[derive(Debug, Clone)]
pub struct PackItem {
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
    pub symbol: Option<String>,
    pub kind: Option<String>,
    pub score: f64,
    pub text: String,
    pub tokens: usize,
    /// Set when the text was compressed — resolves via `imc retrieve <id>`.
    pub ccr_id: Option<String>,
    /// Why this item was included.
    pub why: String,
}

/// A memory that bears on the query.
#[derive(Debug, Clone)]
pub struct PackMemory {
    pub category: String,
    pub content: String,
    pub score: f64,
    pub source: String,
}

/// The assembled bundle.
#[derive(Debug, Clone)]
pub struct Pack {
    pub query: String,
    pub budget: usize,
    pub used: usize,
    pub items: Vec<PackItem>,
    pub memories: Vec<PackMemory>,
    /// True when the budget cut retrieval short.
    pub truncated: bool,
}

impl Pack {
    pub fn remaining(&self) -> usize {
        self.budget.saturating_sub(self.used)
    }
}

/// Options controlling one pack run.
#[derive(Debug, Clone)]
pub struct PackOptions {
    /// Total token budget for the rendered bundle.
    pub budget: usize,
    /// Candidate chunks to consider before applying the budget.
    pub candidates: usize,
    /// Enables the vector leg.
    pub vector: bool,
    /// Include project memories.
    pub memories: bool,
    /// Graph facts per symbol (0 disables graph expansion).
    pub graph_facts: usize,
}

impl Default for PackOptions {
    fn default() -> Self {
        Self {
            budget: 6_000,
            candidates: 12,
            vector: true,
            memories: true,
            graph_facts: 4,
        }
    }
}

/// Build a pack for `query`.
///
/// Retrieval, memories and graph facts are all gathered first, then the budget
/// is spent in priority order: memories (capped), then code by score, then
/// graph facts attached to items that made the cut. Spending the budget on the
/// highest-scoring code first means a tight budget still returns the best hit
/// rather than a proportional smear of everything.
pub fn build(
    store: &Store,
    embedder: Option<&dyn imc_embed::Embedder>,
    query: &str,
    opts: &PackOptions,
    ccr_db: Option<&std::path::Path>,
) -> Result<Pack> {
    let mut memory_tokens = 0usize;

    // --- memories ---------------------------------------------------------
    let mut memories: Vec<PackMemory> = Vec::new();
    if opts.memories {
        let mem_budget = (opts.budget as f64 * MEMORY_SHARE) as usize;
        if let Ok(hits) = imc_memory::recall(store, embedder, query, &[], None, None, 5) {
            let mut spent = 0usize;
            for h in hits {
                let tokens = imc_proxy::compression::estimate_tokens(&h.memory.content);
                if spent + tokens > mem_budget.max(60) {
                    break;
                }
                spent += tokens;
                memories.push(PackMemory {
                    category: h.memory.category.clone(),
                    content: h.memory.content.clone(),
                    score: h.score,
                    source: h.memory.source.clone(),
                });
            }
            memory_tokens = spent;
        }
    }

    // --- retrieval --------------------------------------------------------
    let hits: Vec<SearchHit> = imc_search::search(
        store,
        embedder,
        query,
        &imc_search::SearchOptions {
            limit: opts.candidates,
            vector: opts.vector,
        },
    )?;

    let graph_budget = (opts.budget as f64 * GRAPH_SHARE) as usize;
    let mut used = memory_tokens;
    let mut graph_spent = 0usize;
    let mut items: Vec<PackItem> = Vec::new();
    let mut truncated = false;

    let mut ccr = match ccr_db {
        Some(path) => imc_proxy::compression::ccr_store::CcrStore::open(path).ok(),
        None => None,
    };

    for (rank, hit) in hits.iter().enumerate() {
        let mut text = hit.snippet.clone();
        if text.trim().is_empty() {
            text = hit.text.clone();
        }

        // Compress each candidate on its own so the budget is spent on
        // information, not on repeated boilerplate inside one snippet.
        let compressed = imc_proxy::compression::compress(&text);
        let mut ccr_id = None;
        if compressed.compressed_tokens < compressed.original_tokens && compressed.saved_tokens > 0 {
            text = compressed.compressed.clone();
            if let Some(store_ccr) = ccr.as_mut() {
                if let Ok(id) = store_ccr.store(
                    imc_proxy::compression::kind_slug(compressed.kind),
                    &compressed.original,
                    &compressed.compressed,
                    compressed.original_tokens as i64,
                    compressed.compressed_tokens as i64,
                ) {
                    ccr_id = Some(id);
                }
            }
        }

        let mut tokens = imc_proxy::compression::estimate_tokens(&text);

        // Graph facts: what this symbol calls and who calls it. Half the value
        // of a hit is knowing where it sits.
        let mut why = format!("retrieval rank {}", rank + 1);
        let mut graph_note = String::new();
        if opts.graph_facts > 0 {
            if let Some(name) = &hit.sym_name {
                let callees = store.callees(
                    &store
                        .find_symbols_by_name(name, 1)
                        .first()
                        .map(|s| s.id.clone())
                        .unwrap_or_default(),
                );
                let callers = store.callers(name);
                let mut parts: Vec<String> = Vec::new();
                if !callees.is_empty() {
                    parts.push(format!(
                        "calls {}",
                        callees
                            .iter()
                            .take(opts.graph_facts)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !callers.is_empty() {
                    parts.push(format!(
                        "called by {}",
                        callers
                            .iter()
                            .take(opts.graph_facts)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !parts.is_empty() {
                    let note = format!("graph: {}", parts.join(" | "));
                    let note_tokens = imc_proxy::compression::estimate_tokens(&note);
                    // Graph facts have their own slice of the budget so they
                    // cannot crowd out the code they describe.
                    if graph_spent + note_tokens <= graph_budget {
                        graph_spent += note_tokens;
                        graph_note = note;
                        tokens += note_tokens;
                        why = format!("retrieval rank {}; graph-expanded", rank + 1);
                    }
                }
            }
        }

        if used + tokens > opts.budget {
            truncated = true;
            // Keep the first hit even if it alone exceeds the budget: returning
            // nothing is worse than returning something slightly over.
            if !items.is_empty() {
                continue;
            }
        }

        used += tokens;
        if !graph_note.is_empty() {
            text.push('\n');
            text.push_str(&graph_note);
        }
        items.push(PackItem {
            file: hit.file.clone(),
            start_line: hit.start_line,
            end_line: hit.end_line,
            symbol: hit.sym_name.clone(),
            kind: hit.sym_kind.clone(),
            score: hit.score,
            text,
            tokens,
            ccr_id,
            why,
        });
    }

    Ok(Pack {
        query: query.to_string(),
        budget: opts.budget,
        used,
        items,
        memories,
        truncated,
    })
}

/// Render as markdown — the form an agent pastes into its own context.
pub fn render_markdown(pack: &Pack, root: &std::path::Path) -> String {
    let mut out = String::new();
    out.push_str(&format!("# imc pack — {}\n\n", pack.query));
    out.push_str(&format!(
        "project: {}\nbudget: {} tokens, used {} ({:.0}%)\n",
        root.display(),
        pack.budget,
        pack.used,
        if pack.budget > 0 {
            pack.used as f64 / pack.budget as f64 * 100.0
        } else {
            0.0
        }
    ));
    if pack.truncated {
        out.push_str("note: budget reached — lower-ranking hits were dropped, not summarised\n");
    }
    out.push('\n');

    if !pack.memories.is_empty() {
        out.push_str("## Project memory\n\n");
        for m in &pack.memories {
            out.push_str(&format!(
                "- **{}** (score {:.2}, {}) — {}\n",
                m.category, m.score, m.source, m.content
            ));
        }
        out.push('\n');
    }

    out.push_str("## Code\n\n");
    if pack.items.is_empty() {
        // An empty bundle must say so, and say what to try instead — a bare
        // heading leaves the agent thinking the project has no code.
        out.push_str(&format!(
            "_no matches for {:?} in this index._ Try a broader query, or check \
that the project is indexed (`imc .`) and that the term actually appears \
(the lexical leg does not stem).\n\n",
            pack.query
        ));
    }
    for (i, item) in pack.items.iter().enumerate() {
        let kind = item.kind.as_deref().unwrap_or("chunk");
        out.push_str(&format!(
            "### {}. `{}:{}-{}`{}  (score {:.4})\n",
            i + 1,
            item.file,
            item.start_line,
            item.end_line,
            item.symbol
                .as_ref()
                .map(|s| format!(" {s}"))
                .unwrap_or_default(),
            item.score
        ));
        out.push_str(&format!("_{}_ · {} tokens\n\n", item.why, item.tokens));
        out.push_str("```");
        out.push_str(fence_lang(&item.file).unwrap_or(kind));
        out.push('\n');
        out.push_str(strip_leading_location(&item.text, &item.file, item.start_line));
        out.push_str("\n```\n");
        if let Some(id) = &item.ccr_id {
            out.push_str(&format!(
                "full text: `imc retrieve {id}`\n"
            ));
        }
        out.push('\n');
    }

    let handles: Vec<&PackItem> = pack.items.iter().filter(|i| i.ccr_id.is_some()).collect();
    if !handles.is_empty() {
        out.push_str("## Shortened (full text on demand)\n\n");
        for i in &handles {
            out.push_str(&format!(
                "- `{}:{}-{}` -> `imc retrieve {}`\n",
                i.file,
                i.start_line,
                i.end_line,
                i.ccr_id.as_deref().unwrap_or_default()
            ));
        }
        out.push('\n');
    }
    out
}

/// The indexer prefixes chunk text with its own `path:lines` banner. We already
/// print that in the heading, so repeating it inside the code block wastes
/// tokens and breaks the first line of real code.
fn strip_leading_location<'a>(text: &'a str, file: &str, start_line: usize) -> &'a str {
    let mut rest = text;
    if let Some((first, tail)) = rest.split_once('\n') {
        let t = first.trim();
        let is_banner = (t.starts_with(file) && t.contains(':'))
            || t == format!("{file} (lines {start_line}-");
        if is_banner {
            rest = tail;
        }
    }
    rest
}

/// Fence language from the file extension, so a rendered pack highlights
/// correctly. The AST kind (`struct`, `function`) is not a language.
fn fence_lang(file: &str) -> Option<&'static str> {
    let ext = file.rsplit('.').next()?;
    Some(match ext {
        "rs" => "rust",
        "py" => "python",
        "ts" => "typescript",
        "tsx" => "tsx",
        "js" => "javascript",
        "jsx" => "jsx",
        "go" => "go",
        "java" => "java",
        "kt" => "kotlin",
        "rb" => "ruby",
        "php" => "php",
        "cs" => "csharp",
        "c" | "h" => "c",
        "cc" | "cpp" | "hpp" => "cpp",
        "sh" | "bash" => "bash",
        "sql" => "sql",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" => "markdown",
        _ => return None,
    })
}

/// Render as JSON for a tool-calling agent.
pub fn render_json(pack: &Pack, root: &std::path::Path) -> serde_json::Value {
    serde_json::json!({
        "query": pack.query,
        "project": root.display().to_string(),
        "budget": pack.budget,
        "used": pack.used,
        "truncated": pack.truncated,
        "memories": pack.memories.iter().map(|m| serde_json::json!({
            "category": m.category,
            "content": m.content,
            "score": m.score,
            "source": m.source,
        })).collect::<Vec<_>>(),
        "items": pack.items.iter().map(|i| serde_json::json!({
            "file": i.file,
            "start_line": i.start_line,
            "end_line": i.end_line,
            "symbol": i.symbol,
            "kind": i.kind,
            "score": i.score,
            "tokens": i.tokens,
            "why": i.why,
            "ccr_id": i.ccr_id,
            "text": i.text,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaining_is_never_negative() {
        let p = Pack {
            query: "q".into(),
            budget: 100,
            used: 150,
            items: Vec::new(),
            memories: Vec::new(),
            truncated: true,
        };
        assert_eq!(p.remaining(), 0);
    }

    #[test]
    fn markdown_has_provenance_and_line_ranges() {
        let p = Pack {
            query: "auth flow".into(),
            budget: 1000,
            used: 250,
            items: vec![PackItem {
                file: "src/auth.rs".into(),
                start_line: 10,
                end_line: 40,
                symbol: Some("login".into()),
                kind: Some("fn".into()),
                score: 0.42,
                text: "fn login() {}\n".into(),
                tokens: 4,
                ccr_id: Some("abc123".into()),
                why: "retrieval rank 1".into(),
            }],
            memories: vec![PackMemory {
                category: "rule".into(),
                content: "always use the cookie session".into(),
                score: 0.9,
                source: "memory".into(),
            }],
            truncated: false,
        };
        let md = render_markdown(&p, std::path::Path::new("/repo"));
        assert!(md.contains("src/auth.rs:10-40"));
        assert!(md.contains("login"));
        assert!(md.contains("imc retrieve abc123"));
        assert!(md.contains("cookie session"));
        assert!(md.contains("retrieval rank 1"));
        assert!(md.contains("```rust"), "fence language comes from the extension");
        assert!(md.contains("## Shortened"), "handles are summarised at the end");
    }

    #[test]
    fn fence_language_is_guessed_from_extension() {
        assert_eq!(fence_lang("src/a.rs"), Some("rust"));
        assert_eq!(fence_lang("app/main.py"), Some("python"));
        assert_eq!(fence_lang("weird.zzz"), None);
    }

    #[test]
    fn json_round_trips_the_fields_an_agent_needs() {
        let p = Pack {
            query: "q".into(),
            budget: 10,
            used: 5,
            items: vec![PackItem {
                file: "a.rs".into(),
                start_line: 1,
                end_line: 2,
                symbol: None,
                kind: None,
                score: 0.1,
                text: "x".into(),
                tokens: 1,
                ccr_id: None,
                why: "retrieval rank 1".into(),
            }],
            memories: Vec::new(),
            truncated: false,
        };
        let v = render_json(&p, std::path::Path::new("/r"));
        assert_eq!(v["items"][0]["file"], "a.rs");
        assert_eq!(v["used"], 5);
        assert_eq!(v["truncated"], false);
    }
}
