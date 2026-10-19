use imc_cli::index;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand};

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
    // Long form is `--root` (not `--project`) and the field is `root`, because
    // subcommands take Python's `-p/--project <name_or_id>`: sharing either the
    // long name or the clap arg id makes clap panic on a type mismatch between
    // the global `PathBuf` and the per-command `String`.
    /// Project root (defaults to the active project or the current directory).
    #[arg(long = "root", short = 'C', global = true)]
    root: Option<PathBuf>,

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
        /// Number of chunks to retrieve. Python's `-n/--limit`.
        #[arg(long, short = 'l', short_alias = 'n', value_parser = positive_usize)]
        limit: Option<usize>,
        /// Disable the vector leg (BM25 + graph only).
        #[arg(long)]
        no_vector: bool,
        /// Raw hits only: skip the LLM answer (Python's `-r/--raw`).
        #[arg(long, short = 'r')]
        raw: bool,
        /// Target project by registered id or name (Python's `-p/--project`).
        #[arg(long, short = 'p')]
        project: Option<String>,
    },
    /// Show definitions/callees/callers for a symbol.
    Graph { symbol: String },
    /// Compress text using the proxy compression engine (BM25/graph/CCR).
    #[command(name = "compress")]
    Compress {
        /// Text to compress (read from stdin if omitted).
        #[arg(short, long)]
        text: Option<String>,
        /// Print only the compressed text (no JSON metadata).
        #[arg(long, short = 'r')]
        raw: bool,
        /// Store the original in the CCR store and print its retrieval id.
        #[arg(long)]
        store: bool,
    },
    /// Assemble the context an agent needs: retrieval + graph + memory,
    /// compressed to a token budget.
    #[command(name = "pack")]
    Pack {
        /// What the agent wants to know or do.
        query: String,
        /// Token budget for the whole bundle.
        #[arg(long, default_value_t = 6000)]
        budget: usize,
        /// Candidate chunks considered before the budget is applied.
        #[arg(long, default_value_t = 12)]
        candidates: usize,
        /// Disable the vector leg (BM25 + graph only).
        #[arg(long)]
        no_vector: bool,
        /// Leave project memories out.
        #[arg(long)]
        no_memories: bool,
        /// Graph facts attached per symbol (0 disables).
        #[arg(long = "graph", default_value_t = 4)]
        graph_facts: usize,
        /// Target project by registered id or name.
        #[arg(long, short = 'p')]
        project: Option<String>,
    },
    /// Print the original behind a compression handle.
    #[command(name = "retrieve")]
    Retrieve {
        /// Handle id printed by `imc compress --store` or the proxy.
        id: String,
        /// Print the compressed form instead of the original.
        #[arg(long)]
        compressed: bool,
    },
    /// Inspect the compression store (originals kept for retrieval).
    #[command(name = "ccr")]
    Ccr {
        #[command(subcommand)]
        action: CcrAction,
    },
    /// List stored projects (Python's `imc list`).
    #[command(name = "list", aliases = ["projects", "status"])]
    List,
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
        /// Work-memory type: work_fact|work_task|work_method|work_artifact|business_rule.
        #[arg(long = "type", default_value = "work_fact")]
        mem_type: String,
        /// Working scene this memory belongs to.
        #[arg(long, default_value = "")]
        scene: String,
    },
    /// Drop memories whose tracked files no longer exist on disk.
    #[command(name = "memory-prune")]
    MemoryPrune,
    /// Retrieve memories relevant to a query.
    #[command(name = "memory-search")]
    MemorySearch {
        query: String,
        #[arg(long, short = 'l', default_value_t = 5)]
        limit: usize,
        /// File paths in the caller's context — memories tracking them get the
        /// 0.10 overlap boost from Python's `_blended_score`.
        #[arg(long, short = 'p')]
        path: Vec<String>,
        /// Filter by work-memory type: work_fact|work_task|work_method|work_artifact|business_rule.
        #[arg(long)]
        mem_type: Option<String>,
        /// Filter by working scene this memory belongs to.
        #[arg(long)]
        scene: Option<String>,
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
    /// One-shot Claude Code integration: index + .claude/settings + CLAUDE.md + proxy.
    #[command(name = "claude")]
    Claude {
        #[command(subcommand)]
        action: ClaudeAction,
    },
    /// Run / control the LLM proxy that Claude Code talks to.
    #[command(name = "proxy")]
    Proxy {
        #[command(subcommand)]
        action: ProxyAction,
    },
    /// Token-optimised file reader (Claude Code Read replacement). Python ships
    /// the same reader as `imc find` / `imc-read`.
    #[command(alias = "find")]
    Read {
        /// Path to the file to read.
        file: PathBuf,
        /// Print only the outline of symbols.
        #[arg(long)]
        symbols: bool,
        /// Print only one method/function/class by name.
        #[arg(long)]
        method: Option<String>,
        /// Skip N lines from the top.
        #[arg(long)]
        offset: Option<usize>,
        /// Max lines to print.
        #[arg(long, short = 'l')]
        limit: Option<usize>,
        /// Print without compression.
        #[arg(long)]
        raw: bool,
    },
    /// Call-hierarchy tracer: callees (what it calls) or callers (blast radius).
    Trace(TraceArgs),
    /// One-shot resolved context for a file/module (component + deps + tests).
    #[command(alias = "slice")]
    Context {
        /// File path or query target.
        target: String,
        /// Depth of subcomponent resolution (Python's `-d/--depth`, default 1).
        #[arg(long, short = 'd', default_value_t = 1, value_parser = positive_usize)]
        depth: usize,
        /// Target project by registered id or name (Python's `-p/--project`).
        #[arg(long, short = 'p')]
        project: Option<String>,
    },
    /// View or edit ~/.imc_config.yaml (LLM url/key/model, embedding model).
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

/// Flags shared by `trace`, `callers`, `callees` and `blast`. Python gives all
/// four parsers the same options (`main/cli/main.py`), including `-d/--depth`
/// for depth and the `-r/--callers` / `-b/--both` booleans for direction.
#[derive(Args, Debug)]
struct TraceArgs {
    /// Symbol name, `file::qualified::name`, or a `GET /api/path` route target.
    target: String,
    /// Max tree depth (Python default 2).
    #[arg(long, short = 'd', default_value_t = 2, value_parser = positive_usize)]
    depth: usize,
    /// Reverse trace: who calls this symbol (Python's `-r/--callers`).
    #[arg(long, short = 'r', visible_aliases = ["reverse", "up", "callers"])]
    callers: bool,
    /// Trace callees of this symbol (forward execution flow, the default).
    #[arg(long, short = 'c')]
    callees: bool,
    /// Trace callees and callers (Python's `-b/--both`).
    #[arg(long, short = 'b')]
    both: bool,
    /// Keep builtins / trivial accessors in the output.
    #[arg(long, short = 'a')]
    all: bool,
    /// Disambiguate which file defines the symbol.
    #[arg(long, short = 'f')]
    file: Option<String>,
    /// Direction spelled out, instead of the booleans above.
    #[arg(long, value_parser = ["callees", "callers", "both"])]
    direction: Option<String>,
    /// Target project by registered id or name (Python's `-p/--project`).
    #[arg(long, short = 'p')]
    project: Option<String>,
}

#[derive(Subcommand, Debug)]
enum ClaudeAction {
    /// Index the project, wire .claude/settings.local.json + CLAUDE.md, start the proxy.
    Setup {
        #[arg(long, short = 'f')]
        force: bool,
        /// Proxy port (default 12345).
        #[arg(long, default_value_t = 12345)]
        port: u16,
    },
    /// Write settings + CLAUDE.md without indexing.
    Wire {
        #[arg(long, default_value_t = 12345)]
        port: u16,
    },
    /// Start the proxy in the background.
    Start {
        #[arg(long, default_value_t = 12345)]
        port: u16,
    },
    /// Stop the background proxy.
    Stop,
    /// Show proxy status.
    Status,
}

#[derive(Subcommand, Debug)]
enum ProxyAction {
    /// Serve the proxy in the foreground (used by `imc claude start`).
    Serve {
        #[arg(long, default_value_t = 0)]
        port: u16,
    },
    /// Show proxy status.
    Status,
    /// Stop the background proxy.
    Stop,
}

#[derive(Subcommand, Debug)]
enum CcrAction {
    /// Totals: records, tokens in/out, overall ratio, DB size.
    Stats,
    /// Recent records, newest first.
    List {
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
        /// Filter by kind: json_array|source_code|build_log|git_diff|plain_text.
        #[arg(long)]
        kind: Option<String>,
    },
    /// Delete everything (the originals are gone, handles stop resolving).
    Clear,
    /// Drop records older than N days.
    Prune {
        #[arg(long, default_value_t = 30)]
        days: i64,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigAction {
    /// Print the config file path in use (new, or legacy if only that exists).
    Path,
    /// Move an old ~/.indexer_config.yaml to ~/.imc_config.yaml (one-time).
    Migrate,
    /// Get a dotted key, e.g. llm.base_url or embedding.model (api_key masked).
    Get { key: String },
    /// Set a dotted key: imc config set llm.base_url http://localhost:11434/v1
    Set { key: String, value: String },
    /// List all known settings (secrets masked).
    List,
    /// Probe the configured endpoint (POST {base_url}/embeddings) and report.
    Test,
    /// Interactive setup: prompt for llm.base_url, llm.api_key, llm.model and
    /// embedding.model, then write ~/.imc_config.yaml.
    Setup,
}

fn resolve_root(cfg: &Config, project: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = project {
        // A raw `No such file or directory (os error 2)` from `canonicalize`
        // does not say which argument was wrong.
        return std::fs::canonicalize(&p)
            .map_err(|e| anyhow::anyhow!("project root {} does not exist ({e})", p.display()));
    }
    let cwd = std::env::current_dir()?;
    // Python resolves the project by walking up from the cwd until a registered
    // project matches (main/project/manager.py), so running from a subdirectory
    // still answers for the project that contains it. The active-project
    // pointer is this port's addition and only decides when nothing matches.
    let registry = imc_core::config::load_registry(cfg);
    let mut dir: Option<&std::path::Path> = Some(cwd.as_path());
    while let Some(d) = dir {
        if let Some(p) = registry
            .iter()
            .find(|p| std::path::Path::new(&p.path) == d)
            .map(|p| PathBuf::from(&p.path))
        {
            return Ok(p);
        }
        dir = d.parent();
    }
    if let Some(p) = cfg.active_project() {
        if p.exists() {
            return Ok(p);
        }
    }
    Ok(cwd)
}

/// Python's `positive_int`: argparse rejects `--limit 0` / `--depth 0` with
/// rc=2, and clap should reject them identically rather than clamp silently.
fn positive_usize(s: &str) -> std::result::Result<usize, String> {
    let n: usize = s
        .parse()
        .map_err(|_| format!("invalid integer: {s:?}"))?;
    if n == 0 {
        return Err("must be at least 1".to_string());
    }
    Ok(n)
}

/// Python's `-p/--project <name_or_id>`: a registered project id or name, or
/// (as a convenience) a directory path. `base` is the root the global `-C`
/// resolved to.
fn root_with_project_ref(cfg: &Config, base: &Path, spec: Option<&str>) -> Result<PathBuf> {
    let Some(spec) = spec else {
        return Ok(base.to_path_buf());
    };
    if let Some(p) = imc_core::config::load_registry(cfg)
        .into_iter()
        .find(|p| p.id == spec || p.name == spec)
    {
        return Ok(PathBuf::from(p.path));
    }
    let as_path = PathBuf::from(spec);
    if as_path.is_dir() {
        return Ok(std::fs::canonicalize(as_path)?);
    }
    bail!(
        "no project {spec:?}: not a registered id/name (see `imc projects`) and not a directory"
    )
}

/// `trace`, plus the `callers` / `callees` / `blast` shorthands Python also
/// ships: they take the same flags and only preset the direction.
fn run_trace_arm(
    cfg: &Config,
    base_root: &Path,
    args: &TraceArgs,
    json: bool,
) -> Result<()> {
    let root = root_with_project_ref(cfg, base_root, args.project.as_deref())?;
    let db = cfg.project_db(&root);
    let indexed = db.exists();
    let store = if indexed { store::open(&db)? } else { store::open(&cfg.data_dir.join("empty.db"))? };
    // Python's ASTTracer always builds a bridge for an indexed project
    // (tracer.py:98); without it `GET /api/x` and frontend call targets
    // resolve to nothing.
    let bridge = imc_cli_extra::endpoint_bridge::EndpointRouteBridge::new(&root, Some(&store));
    let tracer =
        imc_cli_extra::adapter::ImcTracer::new(&store, &root, indexed).with_bridge(&bridge);

    let mut opts = imc_cli_extra::trace_cmd::TraceOptions::new(args.target.clone());
    opts.depth = args.depth;
    opts.show_all = args.all;
    opts.json_output = json;
    opts.file_filter = args.file.clone();
    // Direction: --both wins, then --callers, then --callees (the default).
    if args.both {
        opts.both = true;
    } else if args.callers {
        opts.callers = true;
    }

    match imc_cli_extra::trace_cmd::run_trace(&tracer, &opts) {
        Ok(out) => {
            if let Some(js) = out.json {
                println!("{js}");
            } else {
                let text = &out.text;
                print!("{text}");
                if !out.text.is_empty() && !out.text.ends_with('\n') {
                    println!();
                }
            }
        }
        Err(e) => {
            for m in e.messages() {
                eprintln!("{m}");
            }
            std::process::exit(e.exit_code());
        }
    }
    Ok(())
}

/// The human-readable hit listing (`-r/--raw`).
///
/// Python printed the chunk plus its `AST Context:` block; the equivalent here
/// is the `// graph:` evidence the indexer appended, which only `--json`
/// consumers used to see.
fn print_hits(hits: &[imc_core::types::SearchHit], query: &str) {
    for (i, h) in hits.iter().enumerate() {
        let kind = h.sym_kind.as_deref().unwrap_or("chunk");
        println!(
            "\n{}. [{}] {}:{}-{}  (score {:.4})",
            i + 1,
            kind,
            h.file,
            h.start_line,
            h.end_line,
            h.score
        );
        if let Some(name) = &h.sym_name {
            println!("   symbol: {name}");
        }
        println!("{}", indent(&h.snippet, "   "));
        if let Some((_, evidence)) = h.text.split_once("// graph: ") {
            println!("{}", indent(evidence, "   // graph: "));
        }
    }
    if hits.is_empty() {
        println!("no matches for {query:?}");
    }
}

fn main() -> Result<()> {
    // Rust ignores SIGPIPE, so `imc search ... | head` prints a panic
    // ("failed printing to stdout: Broken pipe") instead of exiting quietly.
    // Restoring the default disposition makes the process die on SIGPIPE like
    // any other Unix filter.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    // Allow `imc <path>` as a shortcut for `imc index <path>`.
    let mut args: Vec<String> = std::env::args().collect();
    if args.len() == 2 {
        let p = &args[1];
        if !p.starts_with('-') && std::path::Path::new(p).exists() {
            args.insert(1, "index".to_string());
        }
    }
    let cli = Cli::parse_from(&args);
    let cfg = Config::default();
    cfg.ensure_dirs()?;
    let root = resolve_root(&cfg, cli.root.clone())?;

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

            // Python's index.py prunes memories pointing at deleted files on
            // every non-forced run; keep that behaviour (`imc index --force`
            // already wipes the store via the caller).
            let mut pruned = 0usize;
            if !*force {
                if let Ok(store) = store::open(&cfg.project_db(&target)) {
                    pruned = imc_memory::prune_stale(&store, &target).unwrap_or(0);
                }
            }

            if cli.json {
                let mut v = serde_json::to_value(&stats)?;
                if let Some(o) = v.as_object_mut() {
                    o.insert("memories_pruned".into(), serde_json::json!(pruned));
                }
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                index::report(&stats);
                if pruned > 0 {
                    println!("  memories pruned: {pruned}");
                }
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
            raw,
            project,
        } => {
            let root = root_with_project_ref(&cfg, &root, project.as_deref())?;
            let db = cfg.project_db(&root);
            if !db.exists() {
                bail!("project {} is not indexed. Run `imc index {}` first", root.display(), root.display());
            }
            let store = store::open(&db)?;
            let embedder = imc_embed::build(&cfg.embedding);

            // `query | directive` — the Python CLI splits the pipe first, then
            // resolve_search_limit decides the chunk budget from both halves.
            let (search_query, directive) = match query.split_once('|') {
                Some((q, d)) => (q.trim().to_string(), d.trim().to_string()),
                None => (query.clone(), String::new()),
            };
            let effective = imc_support::limits::resolve_search_limit(
                &search_query,
                &directive,
                limit.map(|v| v as i64),
            )
            .clamp(1, 20) as usize;
            let opts = imc_search::SearchOptions {
                limit: effective.clamp(1, 50),
                vector: !*no_vector,
            };
            let hits = imc_search::search(&store, embedder.as_deref(), &search_query, &opts)?;
            if cli.json || *raw {
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&hits)?);
                } else {
                    print_hits(&hits, query);
                }
            } else if hits.is_empty() {
                // Python prints this (and skips the LLM) when retrieval is empty.
                println!("No results found.");
            } else {
                let context = imc_cli::agentic::context_from_hits(&hits);
                match imc_cli::agentic::ask(&cfg, &search_query, &context)? {
                    imc_cli::agentic::AskOutcome::Answer(answer) => {
                        println!("{answer}");
                        println!();
                    }
                    imc_cli::agentic::AskOutcome::Unavailable(why) => {
                        eprintln!("[imc search] {why}; showing raw hits (`-r/--raw` to skip the LLM)");
                        print_hits(&hits, query);
                    }
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
                    bail!("symbol {symbol:?} not found in index");
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
        Command::Compress { text, raw, store } => {
            let input: String = match text {
                Some(t) => t.clone(),
                None => {
                    let mut stdin = String::new();
                    std::io::stdin().read_to_string(&mut stdin).unwrap();
                    // Preserve the input byte-for-byte: the CCR store must be
                    // able to hand back exactly what came in.
                    stdin
                }
            };
            let result = imc_proxy::compression::compress(&input);
            let ccr_id = if *store {
                let db = imc_proxy::compression::context::ccr_db_path();
                match imc_proxy::compression::ccr_store::CcrStore::open(&db) {
                    Ok(mut s) => match s.store(
                        imc_proxy::compression::kind_slug(result.kind),
                        &result.original,
                        &result.compressed,
                        result.original_tokens as i64,
                        result.compressed_tokens as i64,
                    ) {
                        Ok(id) => Some(id),
                        Err(e) => {
                            eprintln!("could not store original: {e}");
                            None
                        }
                    },
                    Err(e) => {
                        eprintln!("could not open ccr store: {e}");
                        None
                    }
                }
            } else {
                None
            };
            if *raw {
                print!("{}", result.compressed);
                if let Some(id) = &ccr_id {
                    eprintln!("\n[imc] original stored as {id}");
                }
            } else {
                let mut v = serde_json::to_value(&result)?;
                if let Some(id) = &ccr_id {
                    v["id"] = serde_json::json!(id);
                }
                println!("{}", serde_json::to_string(&v)?);
            }
        }
        Command::Pack {
            query,
            budget,
            candidates,
            no_vector,
            no_memories,
            graph_facts,
            project,
        } => {
            let root = root_with_project_ref(&cfg, &root, project.as_deref())?;
            let db = cfg.project_db(&root);
            if !db.exists() {
                bail!(
                    "project {} is not indexed. Run `imc {}` first",
                    root.display(),
                    root.display()
                );
            }
            let store = store::open(&db)?;
            let embedder = imc_embed::build(&cfg.embedding);
            let opts = imc_cli::pack::PackOptions {
                budget: *budget,
                candidates: *candidates,
                vector: !*no_vector,
                memories: !*no_memories,
                graph_facts: *graph_facts,
            };
            let pack = imc_cli::pack::build(
                &store,
                embedder.as_deref(),
                query,
                &opts,
                Some(&imc_proxy::compression::context::ccr_db_path()),
            )?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&imc_cli::pack::render_json(&pack, &root))?
                );
            } else {
                print!("{}", imc_cli::pack::render_markdown(&pack, &root));
            }
        }
        Command::Retrieve { id, compressed } => {
            let db = imc_proxy::compression::context::ccr_db_path();
            let store = imc_proxy::compression::ccr_store::CcrStore::open(&db)?;
            let row = store.conn.query_row(
                "SELECT original, compressed FROM ccr_originals WHERE id = ?1",
                [id.as_str()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            );
            match row {
                Ok((original, comp)) => print!("{}", if *compressed { comp } else { original }),
                Err(_) => {
                    bail!(
                        "no compression record with id `{id}` in {} (handles expire when the store is pruned)",
                        db.display()
                    );
                }
            }
        }
        Command::Ccr { action } => {
            let db = imc_proxy::compression::context::ccr_db_path();
            let mut store = imc_proxy::compression::ccr_store::CcrStore::open(&db)?;
            match action {
                CcrAction::Stats => {
                    let entries = store.list(None, usize::MAX.min(1_000_000))?;
                    let total_orig: i64 = entries.iter().map(|e| e.orig_tokens).sum();
                    let total_comp: i64 = entries.iter().map(|e| e.comp_tokens).sum();
                    let ratio = if total_orig > 0 {
                        total_comp as f64 / total_orig as f64
                    } else {
                        1.0
                    };
                    let size = std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0);
                    if cli.json {
                        println!(
                            "{}",
                            serde_json::to_string(&serde_json::json!({
                                "db": db.display().to_string(),
                                "records": entries.len(),
                                "original_tokens": total_orig,
                                "compressed_tokens": total_comp,
                                "saved_tokens": total_orig - total_comp,
                                "compression_ratio": ratio,
                                "db_bytes": size,
                            }))?
                        );
                    } else {
                        println!("ccr:       {}", db.display());
                        println!("records:   {}", entries.len());
                        println!("tokens:    {total_orig} -> {total_comp} (saved {})", total_orig - total_comp);
                        println!("ratio:     {:.3}", ratio);
                        println!("db size:   {:.1} MB", size as f64 / 1_048_576.0);
                    }
                }
                CcrAction::List { limit, kind } => {
                    let entries = store.list(kind.as_deref(), *limit)?;
                    if cli.json {
                        let rows: Vec<_> = entries
                            .iter()
                            .map(|e| {
                                serde_json::json!({
                                    "id": e.id, "kind": e.kind,
                                    "orig_tokens": e.orig_tokens,
                                    "comp_tokens": e.comp_tokens,
                                    "created_at": e.created_at,
                                })
                            })
                            .collect();
                        println!("{}", serde_json::to_string(&rows)?);
                    } else {
                        println!("{:<18} {:<12} {:>8} {:>8} {:>7}", "id", "kind", "in", "out", "saved");
                        for e in &entries {
                            println!(
                                "{:<18} {:<12} {:>8} {:>8} {:>6.0}%",
                                e.id,
                                e.kind,
                                e.orig_tokens,
                                e.comp_tokens,
                                if e.orig_tokens > 0 {
                                    (e.orig_tokens - e.comp_tokens) as f64 / e.orig_tokens as f64 * 100.0
                                } else {
                                    0.0
                                }
                            );
                        }
                    }
                }
                CcrAction::Clear => {
                    let n = store.count()?;
                    store.conn.execute("DELETE FROM ccr_originals", [])?;
                    println!("cleared {n} record(s) from {}", db.display());
                }
                CcrAction::Prune { days } => {
                    let cutoff = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64
                        - days * 86_400;
                    let n = store.prune_before(cutoff)?;
                    println!("pruned {n} record(s) older than {days} day(s)");
                }
            }
        }
        Command::List => {
            let projects = imc_core::config::load_registry(&cfg);
            if cli.json {
                let mut out = Vec::new();
                for p in &projects {
                    let mut entry = serde_json::json!({
                        "id": p.id,
                        "name": p.name,
                        "path": p.path,
                    });
                    if let Ok(store) = store::open(&cfg.project_db(std::path::Path::new(&p.path))) {
                        let rows = store.stats();
                        let mut counts = serde_json::Map::new();
                        for (k, v) in rows {
                            counts.insert(k, serde_json::json!(v));
                        }
                        entry["counts"] = serde_json::Value::Object(counts);
                        // Disk size of the index DB file.
                        let db_path = cfg.project_db(std::path::Path::new(&p.path));
                        if let Ok(meta) = std::fs::metadata(&db_path) {
                            entry["db_size_mb"] = serde_json::json!(meta.len() as f64 / 1_048_576.0);
                        }
                    }
                    out.push(entry);
                }
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                println!("{:<36} {:<48} {:>6} {:>7} {:>8} {:>10} {:>8}", "id", "path", "files", "symbols", "edges", "chunks", "memories");
                for p in projects {
                    let mut files = "-".to_string();
                    let mut symbols = "-".to_string();
                    let mut edges = "-".to_string();
                    let mut chunks = "-".to_string();
                    let mut memories = "-".to_string();
                    let mut db_mb = 0.0;
                    if let Ok(store) = store::open(&cfg.project_db(std::path::Path::new(&p.path))) {
                        for (k, v) in store.stats() {
                            match k.as_str() {
                                "files" => files = v.to_string(),
                                "symbols" => symbols = v.to_string(),
                                "edges" => edges = v.to_string(),
                                "chunks" => chunks = v.to_string(),
                                _ => {}
                            }
                        }
                        // Memory count: count rows in memories table if present.
                        if let Ok(n) = store.memory_count() {
                        memories = n.to_string();
                    }
                    }
                    if let Ok(meta) = std::fs::metadata(cfg.project_db(std::path::Path::new(&p.path))) {
                        db_mb = meta.len() as f64 / 1_048_576.0;
                    }
                    println!(
                        "{:<36} {:<48} {:>6} {:>7} {:>8} {:>10} {:>8} {:>7.2}MB",
                        p.id,
                        p.path,
                        files,
                        symbols,
                        edges,
                        chunks,
                        memories,
                        db_mb,
                    );
                }
            }
        }
        Command::MemoryAdd {
            content,
            category,
            path,
            tag,
            importance,
            mem_type,
            scene,
        } => {
            let db = cfg.project_db(&root);
            let store = store::open(&db)?;
            let embedder = imc_embed::build(&cfg.embedding);
            match imc_memory::add_typed(
                &store,
                &content,
                imc_memory::MemoryType::from_str(mem_type),
                scene,
                &category,
                &path,
                &tag,
                *importance,
                "manual",
            )? {
                Some(id) => {
                    imc_memory::embed_and_store(&store, embedder.as_deref(), id, &content)?;
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
        Command::MemorySearch { query, limit, path, mem_type, scene } => {
            let db = cfg.project_db(&root);
            let store = store::open(&db)?;
            let embedder = imc_embed::build(&cfg.embedding);
            let hits = imc_memory::recall(
                &store,
                embedder.as_deref(),
                query,
                path,
                mem_type.as_deref(),
                scene.as_deref(),
                *limit,
            )?;
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
        Command::MemoryPrune => {
            let db = cfg.project_db(&root);
            let store = store::open(&db)?;
            let pruned = imc_memory::prune_stale(&store, &root)?;
            println!("pruned {pruned} stale memories referencing deleted files");
        }
        Command::MemoryList { limit } => {
            let db = cfg.project_db(&root);
            let store = store::open(&db)?;
            let items = imc_memory::list(&store, *limit)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else {
                for m in items {
                    let scene = if m.scene.is_empty() {
                        String::new()
                    } else {
                        format!(" @{}", m.scene)
                    };
                    println!(
                        "#{:<4} [{}] imp={:.2} uses={}{}  {}",
                        m.id,
                        m.mem_type.as_str(),
                        m.importance,
                        m.use_count,
                        scene,
                        m.content.lines().next().unwrap_or("")
                    );
                }
            }
        }
        Command::MemoryDelete { id } => {
            let db = cfg.project_db(&root);
            let store = store::open(&db)?;
            let removed = imc_memory::delete(&store, *id)?;
            println!("memory #{id} deleted: {removed}");
        }
        Command::Trace(a) => run_trace_arm(&cfg, &root, a, cli.json)?,
        Command::Context { target, depth, project } => {
            let root = root_with_project_ref(&cfg, &root, project.as_deref())?;
            // Python's `get_feature_context(..., depth)` never reads `depth`
            // (main/cli/commands/context.py:555-681 — only the signature and
            // the help text mention it), so the flag is accepted and ignored,
            // exactly like upstream. It is validated (`>= 1`) so a typo cannot
            // pass silently the way `-d callers` used to.
            let _ = depth;
            let db = cfg.project_db(&root);
            let store = if db.exists() { Some(store::open(&db)?) } else { None };
            let out = imc_find::context::get_feature_context(target, &root, store.as_ref());
            if out.is_empty() {
                bail!("no context resolved for {target:?}");
            }
            // Python used print(), which terminates the dossier with a newline.
            println!("{out}");
        }
        Command::Claude { action } => match action {
            ClaudeAction::Setup { force, port } => {
                let lines = imc_cli::claude::setup_and_start(&cfg, &root, *port, *force)?;
                if cli.json {
                    println!("{}", serde_json::json!({"ok": true, "log": lines}));
                } else {
                    for l in &lines {
                        println!("  ✓ {l}");
                    }
                }
            }
            ClaudeAction::Wire { port } => {
                let s = imc_cli::claude::write_claude_settings(&root, *port)?;
                let (md, changed) = imc_cli::claude::write_claude_md(&root)?;
                println!(
                    "wrote {} and {}{}",
                    s.display(),
                    md.display(),
                    if changed { " (updated)" } else { "" }
                );
            }
            ClaudeAction::Start { port } => {
                let state = imc_cli::claude::start_proxy(&cfg, *port)?;
                println!("proxy started: pid={} http://127.0.0.1:{}", state["pid"], state["port"]);
            }
            ClaudeAction::Stop => match imc_cli::claude::stop_proxy(&cfg)? {
                Some(p) => println!("proxy stopped (was :{p})"),
                None => println!("no proxy state file — nothing to stop"),
            },
            ClaudeAction::Status => {
                let s = imc_cli::claude::proxy_status(&cfg);
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&s).unwrap_or_default());
                } else {
                    match s {
                        Some(v) => println!(
                            "proxy: {} (pid={}, port={})",
                            if v["running"] == true { "RUNNING" } else { "NOT REACHABLE" },
                            v["state"]["pid"],
                            v["state"]["port"]
                        ),
                        None => println!("proxy: not started"),
                    }
                }
            }
        },
        Command::Proxy { action } => match action {
            ProxyAction::Serve { port } => {
                let bin = imc_cli::claude::proxy_bin()?;
                let status = std::process::Command::new(bin)
                    .args(["--port", &port.to_string()])
                    .status()?;
                if !status.success() {
                    std::process::exit(status.code().unwrap_or(1));
                }
            }
            ProxyAction::Status => {
                let s = imc_cli::claude::proxy_status(&cfg);
                println!("{}", serde_json::to_string_pretty(&s).unwrap_or_default());
            }
            ProxyAction::Stop => {
                match imc_cli::claude::stop_proxy(&cfg)? {
                    Some(p) => println!("proxy stopped (was :{p})"),
                    None => println!("no proxy state file"),
                };
            }
        },
        Command::Read { file, symbols, method, offset, limit, raw } => {
            // The reader opens the path as given, which is cwd-relative like
            // Python (where the cwd is always inside the project). With an
            // explicit `-C <root>` the cwd may be elsewhere, and the instruction
            // block tells agents to pass `-C`, so fall back to the project root
            // when the path does not exist relative to the cwd.
            let resolved = {
                let given = Path::new(file);
                if given.exists() {
                    given.to_path_buf()
                } else {
                    let under_root = root.join(given);
                    if under_root.exists() {
                        under_root
                    } else {
                        given.to_path_buf()
                    }
                }
            };
            let file = resolved.to_string_lossy().to_string();
            let opts = imc_find::find::FindOptions {
                limit: *limit,
                offset: *offset,
                raw: *raw,
                // Leak only the --method argument (CLI-lifetime string).
                method: method
                    .as_deref()
                    .map(|m| &*Box::leak(m.to_string().into_boxed_str())),
                symbols: *symbols,
            };
            // `--json read` must stay machine-readable (the global --json flag
            // promises JSON for every command).
            if cli.json && *symbols && !*raw {
                let path = Path::new(&file);
                let text = std::fs::read_to_string(path)
                    .map_err(|e| anyhow::anyhow!("[imc read] Failed to read {}: {e}", path.display()))?;
                let outline = imc_find::find::outline_symbols_struct(&text, path);
                println!("{}", serde_json::to_string_pretty(&outline)?);
                return Ok(());
            }

            let out = imc_find::find::run_find(&file, opts)?;
            print!("{out}");
        }
        Command::Config { action } => {
            use imc_core::settings as st;
            match action {
                ConfigAction::Path => println!("{}", st::active_path().display()),
                ConfigAction::Migrate => match st::migrate_legacy()? {
                    Some(from) => println!("migrated {} -> {}", from.display(), st::path().display()),
                    None => println!("nothing to migrate (no legacy file, or already migrated)"),
                },
                ConfigAction::Get { key } => {
                    let s = st::load();
                    match st::get(&s, &key) {
                        Some(v) if key.ends_with("api_key") || key.ends_with("token") => {
                            println!("{}", st::mask(&v))
                        }
                        Some(v) => println!("{}", v.as_str().map(String::from).unwrap_or_else(|| format!("{v:?}"))),
                        None => {
                            if cli.json {
                                println!("{{}}");
                            } else {
                                println!("(unset) {key}");
                            }
                        }
                    }
                }
                ConfigAction::Set { key, value } => {
                    let mut s = st::load();
                    st::set(&mut s, &key, &value)?;
                    st::save(&s)?;
                    let masked = key.ends_with("api_key") || key.ends_with("token");
                    if cli.json {
                        println!(
                            "{}",
                            serde_json::json!({"key": key, "value": if masked {"***"} else {&value}, "path": st::path()})
                        );
                    } else if masked {
                        println!("{key} = {} (saved to {})", st::mask(&serde_yaml::Value::String(value.clone())), st::path().display());
                    } else {
                        println!("{key} = {value} (saved to {})", st::path().display());
                    }
                }
                ConfigAction::List => {
                    let s = st::load();
                    let pairs: Vec<(&str, Option<serde_yaml::Value>)> = vec![
                        ("llm.base_url", st::get(&s, "llm.base_url")),
                        ("llm.model", st::get(&s, "llm.model")),
                        ("llm.api_key", st::get(&s, "llm.api_key")),
                        ("embedding.model", st::get(&s, "embedding.model")),
                        ("ingest.ignored_dirs", st::get(&s, "ingest.ignored_dirs")),
                    ];
                    if cli.json {
                        let m: serde_json::Map<String, serde_json::Value> = pairs
                            .iter()
                            .map(|(k, v)| {
                                (
                                    (*k).to_string(),
                                    match v {
                                        Some(y) => serde_json::to_value(
                                            y.as_str().map(String::from).unwrap_or_else(|| format!("{y:?}")),
                                        )
                                        .unwrap_or(serde_json::Value::Null),
                                        None => serde_json::Value::Null,
                                    },
                                )
                            })
                            .collect();
                        println!("{}", serde_json::to_string_pretty(&m)?);
                    } else {
                        for (k, v) in pairs {
                            match v {
                                None => println!("{k:20} (unset)"),
                                Some(y) if k.ends_with("api_key") => {
                                    println!("{k:20} {}", st::mask(&y))
                                }
                                Some(y) => println!(
                                    "{k:20} {}",
                                    y.as_str().map(String::from).unwrap_or_else(|| format!("{y:?}"))
                                ),
                            }
                        }
                        println!("\nfile: {}", st::active_path().display());
                    }
                }
                ConfigAction::Setup => {
                    use imc_core::settings as st;
                    let mut s = st::load();
                    println!("
=== imc interactive setup ===");
                    println!("Config file: {}", st::path().display());
                    println!("Leave blank to keep current value.
");

                    // llm.base_url
                    let cur = st::get(&s, "llm.base_url").and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
                    print!("LLM base URL [{cur}]: ");
                    std::io::stdout().flush().ok();
                    let mut line = String::new();
                    std::io::stdin().read_line(&mut line).ok();
                    let v = line.trim();
                    if !v.is_empty() { st::set(&mut s, "llm.base_url", v)?; }

                    // llm.api_key
                    let cur = st::get(&s, "llm.api_key").and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
                    let masked = if cur.is_empty() { "(unset)".to_string() } else { st::mask(&serde_yaml::Value::String(cur.clone())) };
                    print!("LLM API key [{masked}]: ");
                    std::io::stdout().flush().ok();
                    let mut line = String::new();
                    std::io::stdin().read_line(&mut line).ok();
                    let v = line.trim();
                    if !v.is_empty() { st::set(&mut s, "llm.api_key", v)?; }

                    // llm.model (cheap model for imc search)
                    let cur = st::get(&s, "llm.model").and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| "claude-haiku-4.5".to_string());
                    print!("LLM model for imc search (cheap) [{cur}]: ");
                    std::io::stdout().flush().ok();
                    let mut line = String::new();
                    std::io::stdin().read_line(&mut line).ok();
                    let v = line.trim();
                    if !v.is_empty() { st::set(&mut s, "llm.model", v)?; } else if cur.is_empty() {
                        st::set(&mut s, "llm.model", "claude-haiku-4.5")?;
                    }

                    // embedding.model (default text-embedding-3-small)
                    let cur = st::get(&s, "embedding.model").and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| "text-embedding-3-small".to_string());
                    print!("Embedding model [{cur}]: ");
                    std::io::stdout().flush().ok();
                    let mut line = String::new();
                    std::io::stdin().read_line(&mut line).ok();
                    let v = line.trim();
                    if !v.is_empty() { st::set(&mut s, "embedding.model", v)?; } else if cur.is_empty() {
                        st::set(&mut s, "embedding.model", "text-embedding-3-small")?;
                    }

                    st::save(&s)?;
                    println!("
saved to {}", st::path().display());
                    println!("Now run: imc config test  (to verify the endpoint)");
                    println!("Then:  imc claude setup   (to wire a project for Claude Code)");
                }
                ConfigAction::Test => {
                    let cfg2 = imc_core::config::Config::default();
                    let base = cfg2.llm_base_url();
                    let key = cfg2.llm_api_key();
                    let url = format!("{}/embeddings", base.trim_end_matches('/'));
                    let body = serde_json::json!({"model": cfg2.embedding.model, "input": ["ping"]});
                    let agent = ureq::Agent::new_with_defaults();
                    let resp = agent
                        .post(&url)
                        .header("Authorization", format!("Bearer {}", if key.is_empty() { "dummy-key" } else { &key }))
                        .header("Content-Type", "application/json")
                        .send_json(body);
                    match resp {
                        Ok(r) => println!("OK  {url}  status={}", r.status()),
                        Err(e) => {
                            println!("FAIL  {url}\n  {e}");
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
        Command::Learn { file } => {
            let db = cfg.project_db(&root);
            let store = store::open(&db)?;
            let embedder = imc_embed::build(&cfg.embedding);
            let text = std::fs::read_to_string(file)?;
            let active = vec![file.to_string_lossy().to_string()];
            let candidates = imc_memory::extract_candidates(&text, &active);
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
                        let _ = imc_memory::embed_and_store(&store, embedder.as_deref(), id, &c.content);
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

/// Resolve a user-typed symbol to a stored symbol id.
/// Accepts `name`, `file::qualname` (used verbatim), or `name` + `--file`.
fn resolve_symbol_ids(store: &store::Store, symbol: &str, file: Option<&str>) -> Result<Vec<String>> {
    if symbol.contains("::") {
        return Ok(vec![symbol.to_string()]);
    }
    let mut cands = store.find_symbols_by_name(symbol, 25);
    if let Some(f) = file {
        cands.retain(|s| s.file == f || s.file.ends_with(f));
    }
    cands.retain(|s| s.kind.is_definition());
    match cands.len() {
        0 => {
            let all = store.find_symbols_by_name(symbol, 25);
            let hint = if all.is_empty() {
                String::new()
            } else {
                let where_ = all
                    .iter()
                    .map(|s| format!("\n    {}::{} ({}:{})", s.file, s.qualname, s.kind.as_str(), s.line))
                    .collect::<Vec<_>>()
                    .join("");
                format!("\nsymbol exists but is not a definition; pass --file:{where_}")
            };
            bail!("symbol {symbol:?} not found in index{hint}")
        }
        // Names like `search` or `build_index` are common; trace every
        // definition instead of forcing the caller to disambiguate.
        _ => Ok(cands.into_iter().map(|s| s.id).collect()),
    }
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