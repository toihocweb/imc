use imc_cli::{index, mcp};

use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};

use imc_core::config::{upsert_project, Config, ProjectMeta};
use imc_core::hash::md5_hex;
use imc_core::store;

#[derive(Parser, Debug)]
#[command(
    name = "imc",
    about = "IndexMyCode — AST + vector + graph code intelligence (Rust)",
    version
)]
struct Cli {
    /// Project root (defaults to the active project or the current directory).
    #[arg(long, short = 'C', global = true)]
    project: Option<PathBuf>,

    /// Print JSON instead of formatted text.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Index a project (incremental).
    Index {
        path: Option<PathBuf>,
        /// Extra gitignore-style patterns.
        #[arg(long, short = 'i')]
        ignore: Vec<String>,
        /// Re-index everything, ignoring stored hashes.
        #[arg(long)]
        force: bool,
    },
    /// Hybrid search over the index.
    Search {
        query: String,
        #[arg(long, short = 'l')]
        limit: usize,
        /// Disable the vector leg (BM25 + graph only).
        #[arg(long)]
        no_vector: bool,
    },
    /// Show definitions/callees/callers for a symbol.
    Graph { symbol: String },
    /// Print index statistics.
    Stats,
    /// List registered projects.
    Projects,
    /// Run the MCP stdio server.
    Mcp,
    /// Store a durable project memory.
    #[command(name = "memory-add")]
    MemoryAdd {
        content: String,
        #[arg(long, default_value = "note")]
        category: String,
        /// Repeatable file path the memory is about.
        #[arg(long, short = 'p')]
        path: Vec<String>,
        #[arg(long, short = 't')]
        tag: Vec<String>,
        #[arg(long, default_value_t = 0.5)]
        importance: f64,
    },
    /// Retrieve memories relevant to a query.
    #[command(name = "memory-search")]
    MemorySearch {
        query: String,
        #[arg(long, short = 'l', default_value_t = 5)]
        limit: usize,
    },
    /// List stored memories.
    #[command(name = "memory-list")]
    MemoryList {
        #[arg(long, short = 'l', default_value_t = 20)]
        limit: usize,
    },
    /// Delete a memory by id.
    #[command(name = "memory-delete")]
    MemoryDelete { id: i64 },
    /// Extract and store memories from a transcript/markdown file.
    Learn { file: PathBuf },
}

fn resolve_root(cfg: &Config, project: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = project {
        return Ok(std::fs::canonicalize(p)?);
    }
    if let Some(p) = cfg.active_project() {
        if p.exists() {
            return Ok(p);
        }
    }
    Ok(std::env::current_dir()?)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = Config::default();
    cfg.ensure_dirs()?;
    let root = resolve_root(&cfg, cli.project.clone())?;

    match &cli.command {
        Command::Index {
            path,
            ignore,
            force,
        } => {
            let target = match path {
                Some(p) => std::fs::canonicalize(p).unwrap_or(root.clone()),
                None => root.clone(),
            };
            let stats = index::run(&cfg, &target, ignore, *force)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&stats)?);
            } else {
                index::report(&stats);
            }
            let meta = ProjectMeta {
                id: md5_hex(target.to_string_lossy().as_bytes())[..16].to_string(),
                name: target
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "root".into()),
                path: target.to_string_lossy().to_string(),
                indexed_at: Some(now_secs()),
                file_count: Some(stats.total_files as u64),
            };
            upsert_project(&cfg, meta)?;
            cfg.set_active(&target)?;
        }
        Command::Search {
            query,
            limit,
            no_vector,
        } => {
            let db = cfg.project_db(&root);
            if !db.exists() {
                bail!("project {} is not indexed. Run `imc index {}` first", root.display(), root.display());
            }
            let store = store::open(&db)?;
            let embedder = imc_embed::build(&cfg.embedding);
            let opts = imc_search::SearchOptions {
                limit: (*limit).clamp(1, 50),
                vector: !*no_vector,
            };
            let hits = imc_search::search(&store, embedder.as_ref(), query, &opts)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&hits)?);
            } else {
                for (i, h) in hits.iter().enumerate() {
                    let kind = h.sym_kind.as_deref().unwrap_or("chunk");
                    println!("\n{}. [{}] {}:{}-{}  (score {:.4})", i + 1, kind, h.file, h.start_line, h.end_line, h.score);
                    if let Some(name) = &h.sym_name {
                        println!("   symbol: {name}");
                    }
                    println!("{}", indent(&h.snippet, "   "));
                }
                if hits.is_empty() {
                    println!("no matches for {query:?}");
                }
            }
        }
        Command::Graph { symbol } => {
            let db = cfg.project_db(&root);
            let store = store::open(&db)?;
            let syms = store.find_symbols_by_name(symbol, 20);
            if cli.json {
                let payload = serde_json::json!({
                    "definitions": syms,
                    "callees": syms.iter().map(|s| store.callees(&s.id)).collect::<Vec<_>>(),
                    "callers": syms.iter().map(|s| store.callers(&s.name)).collect::<Vec<_>>(),
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                if syms.is_empty() {
                    println!("symbol {symbol:?} not found in index");
                }
                for s in &syms {
                    println!("{} {}  {}:{}-{}", s.kind.as_str(), s.qualname, s.file, s.line, s.end_line);
                    let callees = store.callees(&s.id);
                    if !callees.is_empty() {
                        println!("  → calls: {}", callees.join(", "));
                    }
                    let callers = store.callers(&s.name);
                    if !callers.is_empty() {
                        let short: Vec<String> = callers
                            .iter()
                            .map(|c| c.rsplit("::").next().unwrap_or(c).to_string())
                            .collect();
                        println!("  ← called by: {}", short.join(", "));
                    }
                }
            }
        }
        Command::Stats => {
            let db = cfg.project_db(&root);
            let store = store::open(&db)?;
            let rows = store.stats();
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&serde_json::json!({
                    "root": root.display().to_string(), "db": db.display().to_string(), "counts": rows.iter().map(|(k,v)| serde_json::json!({k.clone(): v})).collect::<Vec<_>>()
                }))?);
            } else {
                println!("project: {}", root.display());
                println!("db:      {}", db.display());
                for (k, v) in rows {
                    println!("{k:>10}: {v}");
                }
            }
        }
        Command::Projects => {
            let projects = imc_core::config::load_registry(&cfg);
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&projects)?);
            } else {
                for p in projects {
                    println!(
                        "{}  {:<24}  files={}",
                        p.id,
                        p.path,
                        p.file_count.map(|c| c.to_string()).unwrap_or_else(|| "-".into())
                    );
                }
            }
        }
        Command::Mcp => mcp::serve(&cfg)?,
        Command::MemoryAdd {
            content,
            category,
            path,
            tag,
            importance,
        } => {
            let db = cfg.project_db(&root);
            let store = store::open(&db)?;
            let embedder = imc_embed::build(&cfg.embedding);
            match imc_memory::add(&store, &content, &category, &path, &tag, *importance, "manual")? {
                Some(id) => {
                    imc_memory::embed_and_store(&store, embedder.as_ref(), id, &content)?;
                    if cli.json {
                        println!("{}", serde_json::json!({"id": id, "stored": true}));
                    } else {
                        println!("stored memory #{id}");
                    }
                }
                None => {
                    if cli.json {
                        println!("{}", serde_json::json!({"stored": false, "reason": "duplicate"}));
                    } else {
                        println!("duplicate memory — nothing stored");
                    }
                }
            }
        }
        Command::MemorySearch { query, limit } => {
            let db = cfg.project_db(&root);
            let store = store::open(&db)?;
            let embedder = imc_embed::build(&cfg.embedding);
            let hits = imc_memory::retrieve(&store, embedder.as_ref(), query, &[], *limit)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&hits)?);
            } else {
                for (i, h) in hits.iter().enumerate() {
                    println!("\n{}. [{}] score={:.3}  {}", i + 1, h.memory.category, h.score, h.memory.source);
                    println!("{}", indent(&h.memory.content, "   "));
                }
                if hits.is_empty() {
                    println!("no memories matched {query:?}");
                }
            }
        }
        Command::MemoryList { limit } => {
            let db = cfg.project_db(&root);
            let store = store::open(&db)?;
            let items = imc_memory::list(&store, *limit)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else {
                for m in items {
                    println!("#{:<4} [{}] imp={:.2} uses={}  {}", m.id, m.category, m.importance, m.use_count, m.content.lines().next().unwrap_or(""));
                }
            }
        }
        Command::MemoryDelete { id } => {
            let db = cfg.project_db(&root);
            let store = store::open(&db)?;
            let removed = imc_memory::delete(&store, *id)?;
            println!("memory #{id} deleted: {removed}");
        }
        Command::Learn { file } => {
            let db = cfg.project_db(&root);
            let store = store::open(&db)?;
            let embedder = imc_embed::build(&cfg.embedding);
            let text = std::fs::read_to_string(file)?;
            let candidates = imc_memory::extract_candidates(&text, &[]);
            let mut stored = 0;
            let summary: Vec<serde_json::Value> = candidates
                .iter()
                .filter_map(|c| {
                    let id = imc_memory::add(
                        &store,
                        &c.content,
                        &c.category,
                        &c.paths,
                        &c.tags,
                        0.6,
                        &c.source,
                    )
                    .ok()
                    .flatten();
                    if let Some(id) = id {
                        let _ = imc_memory::embed_and_store(&store, embedder.as_ref(), id, &c.content);
                        stored += 1;
                    }
                    Some(serde_json::json!({"id": id, "category": c.category, "content": c.content}))
                })
                .collect();
            if cli.json {
                println!("{}", serde_json::json!({"candidates": candidates.len(), "stored": stored, "items": summary}));
            } else {
                println!("extracted {} candidates, stored {stored}", candidates.len());
            }
        }
    }
    Ok(())
}

fn indent(s: &str, pad: &str) -> String {
    s.lines()
        .map(|l| format!("{pad}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}