use imc_cli::index;

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
        limit: Option<usize>,
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
    /// Token-optimised file reader (Claude Code Read replacement).
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
    Trace {
        /// Symbol name or `file::qualified::name`.
        symbol: String,
        /// callees (forward, default) or callers (blast radius).
        #[arg(long, short = 'd', default_value = "callees")]
        direction: String,
        /// Max tree depth (Python default 2).
        #[arg(long, default_value_t = 2)]
        depth: usize,
        /// Keep builtins / trivial accessors in the output.
        #[arg(long)]
        all: bool,
        /// Disambiguate which file defines the symbol.
        #[arg(long, short = 'f')]
        file: Option<String>,
    },
    /// One-shot resolved context for a file/module (component + deps + tests).
    Context {
        /// File path or query target.
        target: String,
    },
    /// View or edit ~/.imc_config.yaml (LLM url/key/model, embedding model).
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
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
    // Rust ignores SIGPIPE, so `imc search ... | head` prints a panic
    // ("failed printing to stdout: Broken pipe") instead of exiting quietly.
    // Restoring the default disposition makes the process die on SIGPIPE like
    // any other Unix filter.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

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
        } => {
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
        Command::MemorySearch { query, limit, path } => {
            let db = cfg.project_db(&root);
            let store = store::open(&db)?;
            let embedder = imc_embed::build(&cfg.embedding);
            let hits = imc_memory::recall(&store, embedder.as_deref(), query, path, *limit)?;
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
        Command::Trace {
            symbol,
            direction,
            depth,
            all,
            file,
        } => {
            let db = cfg.project_db(&root);
            let indexed = db.exists();
            let store = if indexed { store::open(&db)? } else { store::open(&cfg.data_dir.join("empty.db"))? };
            let tracer = imc_cli_extra::adapter::ImcTracer::new(&store, &root, indexed);

            let mut opts = imc_cli_extra::trace_cmd::TraceOptions::new(symbol.clone());
            opts.depth = *depth;
            opts.show_all = *all;
            opts.json_output = cli.json;
            opts.file_filter = file.clone();
            match direction.as_str() {
                "callers" | "caller" | "up" => opts.callers = true,
                "both" => opts.both = true,
                _ => {}
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
        }
        Command::Context { target } => {
            let db = cfg.project_db(&root);
            let store = if db.exists() { Some(store::open(&db)?) } else { None };
            let out = imc_find::context::get_feature_context(target, &root, store.as_ref());
            if out.is_empty() {
                bail!("no context resolved for {target:?}");
            }
            print!("{out}");
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
                let path = std::path::Path::new(&file);
                let text = std::fs::read_to_string(path)
                    .map_err(|e| anyhow::anyhow!("[imc read] Failed to read {}: {e}", path.display()))?;
                let outline = imc_find::find::outline_symbols_struct(&text, path);
                println!("{}", serde_json::to_string_pretty(&outline)?);
                return Ok(());
            }

            let out = imc_find::find::run_find(&file.to_string_lossy(), opts)?;
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