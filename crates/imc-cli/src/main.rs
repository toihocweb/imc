use imc_cli::index;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand};

use imc_core::config::{upsert_project, Config, ProjectMeta};
use imc_core::hash::md5_hex;
use imc_core::store;
use imc_memory;


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

/// Subcommands of `imc capture`. Bare `imc capture` shows the status.
#[derive(Subcommand, Debug)]
enum CaptureAction {
    /// Show whether capture is on and how many payloads are stored.
    Status,
    /// Turn prompt capture on.
    On,
    /// Turn prompt capture off.
    Off,
    /// List the captured payloads, newest first.
    List,
    /// Print one captured payload.
    Show { id: String },
    /// Delete one captured payload.
    Rm { id: String },
    /// Delete every captured payload.
    Clear,
}

/// Subcommands of `imc mcp`. Bare `imc mcp` serves over stdio.
#[derive(Subcommand, Debug)]
enum McpAction {
    /// Register this project so MCP clients discover the imc tools (.mcp.json).
    Install {
        /// Config file to write (defaults to <root>/.mcp.json).
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Stop all running imc background services (proxy and web dashboard).
    #[command(name = "stop")]
    Stop,
    /// Watch the repository for git branch or commit changes and auto re-index.
    #[command(name = "watch")]
    Watch {
        /// Project directory to watch (defaults to current directory).
        path: Option<PathBuf>,
    },
    /// Index a project (incremental). Shortcut: `imc <path>` (e.g. `imc .`).
    #[command(hide = true)]
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
        /// Skip this many hits — page through a result set deliberately
        /// instead of re-running a broad query.
        #[arg(long, default_value_t = 0)]
        offset: usize,
        /// One line per hit: location, symbol and the first line only.
        #[arg(long)]
        concise: bool,
        /// Raw hits only: skip the LLM answer (Python's `-r/--raw`).
        #[arg(long, short = 'r')]
        raw: bool,
        /// Target project by registered id or name (Python's `-p/--project`).
        #[arg(long, short = 'p')]
        project: Option<String>,
        /// When search returns no hits, automatically record the query as a
        /// project memory (work_fact / search_fail) so the next agentic pass
        /// can surface it. Follow up with `imc memory-add` to refine the rule.
        /// Use --no-on-miss to disable.
        #[arg(long = "no-on-miss")]
        no_on_miss: bool,
        /// Rerank candidate chunks using the configured LLM.
        #[arg(long)]
        rerank: bool,
    },
    /// Show definitions/callees/callers for a symbol.
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
        /// Rerank candidate chunks using the configured LLM.
        #[arg(long)]
        rerank: bool,
    },
    /// Generate a compact architectural repository map within a token budget.
    #[command(name = "map", aliases = ["repomap"])]
    Map {
        /// Target project directory (defaults to current directory).
        path: Option<PathBuf>,
        /// Token budget for the repository map (default 1024).
        #[arg(long, short = 't', default_value_t = 1024)]
        tokens: usize,
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
    /// Open Claude Code on this machine, with the index wired in.
    ///
    /// `imc claude` starts the proxy, writes the settings and the rule
    /// file, and hands the terminal to the `claude` binary. Every argument
    /// goes straight through — flags, subcommands and all — so
    /// `imc claude --dangerously-skip-permissions` is
    /// `claude --dangerously-skip-permissions`, and `imc claude agents`
    /// is `claude agents`.
    ///
    /// Nothing is intercepted: the options and the subcommands belong to
    /// Claude Code, and a second set of them here would be a second thing
    /// to keep in sync — and a name collision waiting to happen
    /// (`claude stop`, `claude mcp`, `claude doctor` all exist).
    #[command(
        name = "claude",
        trailing_var_arg = true,
        allow_hyphen_values = true,
        disable_help_flag = true
    )]
    Claude {
        /// Arguments for the `claude` binary. Everything, unchanged.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Index the project and wire it for Claude Code, without opening it.
    #[command(name = "setup", hide = true)]
    Setup {
        #[arg(long, short = 'f')]
        force: bool,
        /// Proxy port (default 12345).
        #[arg(long, default_value_t = 12345)]
        port: u16,
        /// Wire the project without starting the proxy.
        #[arg(long)]
        no_proxy: bool,
    },
    /// Run / control the LLM proxy that Claude Code talks to.
    #[command(name = "proxy", hide = true)]
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
    /// Diagnose environment, config, proxy, and project state; print fix commands.
    Doctor,
    /// MCP server (stdio) exposing imc tools to any MCP client.
    #[command(name = "mcp")]
    Mcp {
        #[command(subcommand)]
        action: Option<McpAction>,
    },
    /// Start the proxy and the dashboard — what a bare `imc` does.
    #[command(name = "start")]
    Start,
    /// Install this build somewhere on PATH (plus the dashboard app).
    #[command(name = "install")]
    Install {
        /// Directory for the binary (default: ~/.local/bin).
        #[arg(long)]
        bin_dir: Option<PathBuf>,
        /// Do not copy the dashboard app.
        #[arg(long = "no-dashboard")]
        no_dashboard: bool,
    },
    /// Update to the latest release, or just check whether one exists.
    #[command(name = "update")]
    Update {
        /// Only check for a new release; do not install it.
        #[arg(long)]
        check: bool,
        /// Re-install the latest release even if already on the same version.
        #[arg(long, short = 'f')]
        force: bool,
    },
    /// Capture the full prompt the proxy receives (see what the model really sees).
    #[command(name = "capture")]
    Capture {
        #[command(subcommand)]
        action: Option<CaptureAction>,
    },
    /// Open the web dashboard, starting it first if it is not running.
    #[command(name = "web")]
    Web {
        /// Port the dashboard listens on.
        #[arg(long, default_value_t = imc_cli::web::DEFAULT_PORT)]
        port: u16,
        /// Print the URL but do not launch a browser.
        #[arg(long = "no-open")]
        no_open: bool,
        /// URL to open (overrides IMC_WEB_URL / localhost). Use for a remote host.
        #[arg(long)]
        url: Option<String>,
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
enum ModelMapAction {
    /// Add or replace one mapping.
    Set { from: String, to: String },
    /// Remove one mapping.
    Rm { from: String },
}

#[derive(Subcommand, Debug)]
enum ProxyAction {
    /// Serve the proxy in the foreground (the background one is started by `imc setup`).
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
    /// List the models the configured endpoint serves (GET {base_url}/models).
    /// The API key stays in the config — it is never echoed.
    Models {
        /// Which list to print: chat models, embedding models, or all.
        #[arg(long, default_value = "chat")]
        kind: String,
    },
    /// Interactive setup: prompt for llm.base_url, llm.api_key, llm.model and
    /// embedding.model, then write ~/.imc_config.yaml.
    Setup,
    /// Model mappings the proxy applies: the model a client asks for → the
    /// model the upstream serves. Without one, the model is forwarded as-is.
    ModelMap {
        #[command(subcommand)]
        action: Option<ModelMapAction>,
    },
}

/// Resolve the project root.
///
/// `use_active` decides whether the globally active project may stand in when
/// the current directory is not inside any registered project. Query commands
/// want that fallback (it is how a shell far from the repo still works), but
/// a command that *creates* state must not: `imc index` in a fresh directory
/// has to index that directory, not re-index whatever project was active.
fn resolve_root_with(
    cfg: &Config,
    project: Option<PathBuf>,
    use_active: bool,
    notify: bool,
) -> Result<PathBuf> {
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
    if use_active {
        if let Some(p) = cfg.active_project() {
            if p.exists() {
                // Say so: answering for a project you are not standing in is
                // surprising, and the result looks like it came from here.
                if notify {
                    eprintln!(
                        "[imc] using the active project {} — the current directory is not \
inside a project; pass -C <path> to choose another",
                        p.display()
                    );
                }
                return Ok(p);
            }
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
    let mut store = if indexed { store::open(&db)? } else { store::open(&cfg.data_dir.join("empty.db"))? };
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
                // The definitions of the target, so a JSON consumer does not
                // have to look them up separately. `imc graph` used to be a
                // separate verb for exactly this.
                let mut v: serde_json::Value = match serde_json::from_str(&js) {
                    Ok(v) => v,
                    Err(_) => {
                        println!("{js}");
                        return Ok(());
                    }
                };
                if let Some(obj) = v.as_object_mut() {
                    let defs = resolve_symbols(&store, &args.target, args.file.as_deref())
                        .unwrap_or_default();
                    obj.insert("definitions".to_string(), serde_json::to_value(defs)?);
                }
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                let text = &out.text;
                print!("{text}");
                if !out.text.is_empty() && !out.text.ends_with('\n') {
                    println!();
                }
            }

            // Vòng 2: nếu trace --callers và số caller vượt ngưỡng, tự ghi
            // business_rule vào memory để lần sau imc pack đưa ra cảnh báo.
            if opts.callers {
                let threshold: usize = std::env::var("IMC_TRACE_BUSINESS_RULE_THRESHOLD")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(5);
                let callers = store.callers(&args.target);
                if callers.len() >= threshold {
                    let meta = imc_core::config::ProjectMeta {
                        id: imc_core::hash::md5_hex(root.to_string_lossy().as_bytes())[..16]
                            .to_string(),
                        name: root
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "root".into()),
                        path: root.to_string_lossy().to_string(),
                        indexed_at: Some(imc_cli::claude::chrono_now()),
                        file_count: None,
                    };
                    let _ = imc_core::config::upsert_project(&cfg, meta);
                    let content = format!(
                        "blast radius: {} has {} callers — run `imc trace {} --callers` before editing",
                        args.target,
                        callers.len(),
                        args.target
                    );
                    let _ = imc_memory::add(
                        &mut store,
                        &content,
                        "business_rule",
                        &[],
                        &[],
                        0.7,
                        "trace_blast_radius",
                    );
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
/// One line per hit: where it is and what it is.
///
/// The full listing prints the whole snippet, which is what a human wants
/// and what an agent does not: the agent needs the location and the symbol,
/// then reads the file itself. `imc search --concise` costs a fraction of
/// the tokens.
fn print_hits_concise(hits: &[imc_core::types::SearchHit], query: &str) {
    for (i, h) in hits.iter().enumerate() {
        let kind = h.sym_kind.as_deref().unwrap_or("chunk");
        let sym = h.sym_name.as_deref().unwrap_or("");
        let head = h.snippet.lines().next().unwrap_or("").trim();
        let head = if head.len() > 100 {
            format!("{}…", &head[..100])
        } else {
            head.to_string()
        };
        println!(
            "{}. [{}] {}:{}-{} {} (score {:.4}) {}",
            i + 1,
            kind,
            h.file,
            h.start_line,
            h.end_line,
            sym,
            h.score,
            head
        );
    }
    if hits.is_empty() {
        println!("no matches for {query:?}");
    }
}

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
    // A bare `imc` starts the proxy and the dashboard: one command to get
    // the whole setup running.
    if args.len() == 1 {
        args.push("start".to_string());
    }
    if args.len() == 2 {
        let p = &args[1];
        if !p.starts_with('-') && std::path::Path::new(p).exists() {
            args.insert(1, "index".to_string());
        }
    }
    // `imc claude` is a passthrough: every argument after it belongs to
    // Claude Code. Nothing is rewritten here — a second set of subcommands
    // under `imc claude` would collide with Claude Code's own (`stop`,
    // `mcp`, `doctor`, ...) and would need keeping in sync for no gain.
    let cli = Cli::parse_from(&args);
    let cfg = Config::default();
    cfg.ensure_dirs()?;
    // `imc index` decides its own root (see the Index arm), so it must not
    // print the "using the active project" notice.
    let notify_active = !matches!(
        &cli.command,
        Command::Index { .. } | Command::Proxy { .. } | Command::Stop | Command::Watch { .. }
    );
    let root = resolve_root_with(&cfg, cli.root.clone(), true, notify_active)?;

    match &cli.command {
        Command::Stop => {
            let proxy_stopped = imc_cli::claude::stop_proxy(&cfg)?;
            let dashboard_stopped = imc_cli::web::stop_dashboard();
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "proxy_stopped": proxy_stopped.is_some(),
                        "proxy_port": proxy_stopped,
                        "dashboard_stopped": dashboard_stopped,
                    })
                );
            } else {
                let mut any = false;
                if let Some(port) = proxy_stopped {
                    println!("✓ proxy stopped (port {port})");
                    any = true;
                }
                if dashboard_stopped {
                    println!("✓ web dashboard stopped");
                    any = true;
                }
                if !any {
                    println!("no running imc services found");
                }
            }
        }
        Command::Watch { path } => {
            let target = match path {
                Some(p) => std::fs::canonicalize(p).unwrap_or(root.clone()),
                None => resolve_root_with(&cfg, None, false, false)?,
            };
            let _ = imc_cli::claude::ensure_project_indexed(&cfg, &target);
            println!(
                "[imc watch] Event-driven git watcher active for {}.\nWatching for branch/commit changes (0% CPU when idle). Press Ctrl-C to stop.",
                target.display()
            );
            let mut handle = imc_index::spawn_git_watcher(cfg.clone())?;
            setup_signal_handler();

            while !STOP_REQUESTED.load(std::sync::atomic::Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            handle.stop();
            println!("\n[imc watch] Stopped.");
        }
        Command::Index {
            path,
            ignore,
            force,
        } => {
            let target = match path {
                Some(p) => std::fs::canonicalize(p).unwrap_or(root.clone()),
                // No explicit path: index the project that contains the cwd, or
                // the cwd itself. The globally active project must not win
                // here — `imc index` in a fresh directory would otherwise
                // re-index an unrelated repository.
                None => resolve_root_with(&cfg, None, false, false)?,
            };
            // Progress goes to stderr: stdout may carry --json.
            use std::io::IsTerminal;
            let tty = std::io::stderr().is_terminal();
            let last = std::sync::Mutex::new(
                std::time::Instant::now() - std::time::Duration::from_secs(5),
            );
            let printer = |p: imc_index::Progress| {
                let label = match p.phase {
                    imc_index::Phase::Scan => "scanning",
                    imc_index::Phase::Parse => "parsing",
                    imc_index::Phase::Embed => "indexing",
                    imc_index::Phase::Finalize => "building index",
                };
                let mut last = match last.lock() {
                    Ok(l) => l,
                    Err(_) => return,
                };
                let now = std::time::Instant::now();
                // Fast on a terminal, calm when the output is piped.
                let min_gap = std::time::Duration::from_millis(if tty { 120 } else { 1000 });
                if now.duration_since(*last) < min_gap && p.done < p.total {
                    return;
                }
                *last = now;
                if p.total > 0 {
                    eprintln!("[imc] {label} {}/{} {}", p.done, p.total, p.current);
                } else {
                    eprintln!("[imc] {label}…");
                }
            };
            let stats = imc_index::run_with_progress(&cfg, &target, ignore, *force, &printer)?;

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
            offset,
            concise,
            raw,
            project,
            no_on_miss,
            rerank,
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
            let want = effective.clamp(1, 50);
            let fetch_limit = if *rerank { (want * 2).clamp(10, 30) } else { want + offset + 1 };
            let opts = imc_search::SearchOptions {
                limit: fetch_limit,
                vector: !*no_vector,
            };
            let mut hits = imc_search::search(&store, embedder.as_deref(), &search_query, &opts)?;

            if *rerank && !hits.is_empty() {
                match imc_cli::agentic::rerank(&cfg, &search_query, &hits)? {
                    imc_cli::agentic::RerankOutcome::Reranked(reranked) => {
                        if !reranked.is_empty() {
                            hits = reranked;
                        }
                    }
                    imc_cli::agentic::RerankOutcome::Unavailable(why) => {
                        eprintln!("[imc search] LLM reranking unavailable ({why}); showing raw hits");
                    }
                }
            }

            let has_more = hits.len() > want + offset;
            hits.truncate(want + offset);
            let skipped = (*offset).min(hits.len());
            hits.drain(0..skipped);
            if *rerank || cli.json || *raw {
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&hits)?);
                } else if *concise {
                    print_hits_concise(&hits, query);
                    if has_more {
                        println!(
                            "\nmore matches exist — narrow the query, or page with --offset {}",
                            skipped + hits.len()
                        );
                    }
                } else {
                    print_hits(&hits, query);
                    if has_more {
                        println!(
                            "\nmore matches exist — narrow the query, or page with --offset {}",
                            skipped + hits.len()
                        );
                    }
                }
            } else if hits.is_empty() {
                // Python prints this (and skips the LLM) when retrieval is empty.
                println!("No results found.");
                if !*no_on_miss {
                    // Auto-record the failed query as a project memory so the
                    // next agentic pass can surface it. The user should follow
                    // up with `imc memory-add` to refine the rule.
                    let meta = imc_core::config::ProjectMeta {
                        id: imc_core::hash::md5_hex(root.to_string_lossy().as_bytes())[..16].to_string(),
                        name: root
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "root".into()),
                        path: root.to_string_lossy().to_string(),
                        indexed_at: Some(imc_cli::claude::chrono_now()),
                        file_count: None,
                    };
                    imc_core::config::upsert_project(&cfg, meta)?;
                    let db = cfg.project_db(&root);
                    let mut store = imc_core::store::open(&db)?;
                    imc_memory::add(&mut store, &query, "work_fact", &[], &[], 0.5, "search_fail")?;
                }
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
            rerank,
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
                rerank: *rerank,
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
        Command::Map { path, tokens } => {
            let target = match path {
                Some(p) => std::fs::canonicalize(p).unwrap_or(root.clone()),
                None => resolve_root_with(&cfg, None, false, false)?,
            };
            let db = cfg.project_db(&target);
            if !db.exists() {
                bail!("project is not indexed yet. Run `imc {}` first.", target.display());
            }
            let store = store::open(&db)?;
            let report = imc_find::repomap::generate_repo_map(&store, *tokens)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", report.text);
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
        Command::Claude { args } => {
            // Everything goes to Claude Code, unchanged: the flags, the
            // subcommands, the prompt. imc only prepares the ground before
            // handing over the terminal.
            imc_cli::claude::run_claude_bin(&cfg, &root, &args)?;
        }
        Command::Setup {
            force,
            port,
            no_proxy,
        } => {
            let lines =
                imc_cli::claude::setup_and_start(&cfg, &root, *port, *force, !*no_proxy)?;
            if cli.json {
                println!("{}", serde_json::json!({"ok": true, "log": lines}));
            } else {
                for l in &lines {
                    println!("  ✓ {l}");
                }
            }
        }
        Command::Proxy { action } => match action {
            ProxyAction::Serve { port } => {
                // The proxy runs in this process — there is no separate
                // daemon binary any more.
                //
                // The state file is written here so that `imc proxy status`
                // and `imc proxy stop` see a proxy started this way, the
                // same as one started in the background. Without it, a
                // foreground proxy was invisible to the two commands that
                // exist to manage it.
                // The bind is the test. A port that is taken fails there,
                // with the real reason, and the state file is left alone —
                // it is the record of the proxy that is already serving.
                //
                // (A "is the port in use?" probe before the bind was wrong
                // twice over: it guessed instead of asking the kernel, and
                // a socket in TIME_WAIT answers a connect long after the
                // port is bindable again. So a proxy that had just died
                // could not be replaced — the new one refused to start
                // because the old one's port still answered.)
                if let Err(e) = imc_cli::claude::write_proxy_state(&cfg, *port, std::process::id())
                {
                    eprintln!("[imc proxy] could not write the state file: {e:#}");
                }
                let result = imc_proxy::run(*port);
                imc_cli::claude::clear_proxy_state(&cfg);
                if let Err(e) = result {
                    eprintln!("[imc proxy] {e:#}");
                    std::process::exit(1);
                }
            }
            ProxyAction::Status => {
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
                    println!("Then:  imc setup   (to index and wire a project for Claude Code)");
                }
                ConfigAction::ModelMap { action } => {
                    // The proxy's own config file: the CLI edits it through the
                    // same helpers the proxy reads it with, so the file stays
                    // byte-for-byte what the proxy expects.
                    use imc_proxy::imc_proxy_types::config_store as cs;
                    match action {
                        Some(ModelMapAction::Set { from, to }) => {
                            let mut cfg3 = cs::read_config_from_disk();
                            let mut mappings = cfg3.model_mappings.clone().unwrap_or_default();
                            mappings.insert(from.clone(), to.clone());
                            cfg3.model_mappings = Some(mappings);
                            cs::write_config_to_disk(&cfg3)?;
                            if cli.json {
                                println!("{}", serde_json::json!({"ok": true, "from": from, "to": to, "path": cs::config_file_path().display().to_string()}));
                            } else {
                                println!("mapped {from} -> {to}");
                                println!("  {}", cs::config_file_path().display());
                            }
                        }
                        Some(ModelMapAction::Rm { from }) => {
                            let mut cfg3 = cs::read_config_from_disk();
                            let mut mappings = cfg3.model_mappings.clone().unwrap_or_default();
                            let existed = mappings.remove(from).is_some();
                            cfg3.model_mappings = Some(mappings);
                            cs::write_config_to_disk(&cfg3)?;
                            if cli.json {
                                println!("{}", serde_json::json!({"ok": existed, "removed": from}));
                            } else if existed {
                                println!("removed {from}");
                            } else {
                                println!("no mapping for {from}");
                            }
                        }
                        None => {
                            let cfg3 = cs::read_config_from_disk();
                            let mappings = cfg3.model_mappings.clone().unwrap_or_default();
                            if cli.json {
                                println!("{}", serde_json::json!({"mappings": mappings, "path": cs::config_file_path().display().to_string()}));
                            } else {
                                println!("config: {}", cs::config_file_path().display());
                                if mappings.is_empty() {
                                    println!("(no mappings — every model is forwarded unchanged)");
                                } else {
                                    for (from, to) in &mappings {
                                        println!("  {from} -> {to}");
                                    }
                                }
                            }
                        }
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
                ConfigAction::Models { kind } => {
                    let cfg2 = imc_core::config::Config::default();
                    let base = cfg2.llm_base_url();
                    if base.trim().is_empty() {
                        eprintln!("no llm.base_url configured — run `imc config set llm.base_url <url>`");
                        std::process::exit(1);
                    }
                    let key = cfg2.llm_api_key();
                    let url = format!("{}/models", base.trim_end_matches('/'));
                    let agent = ureq::Agent::new_with_defaults();
                    let mut req = agent.get(&url);
                    if !key.is_empty() {
                        req = req.header("Authorization", format!("Bearer {key}"));
                    }
                    match req.call() {
                        Ok(mut resp) => {
                            let body = resp.body_mut().read_to_string().unwrap_or_default();
                            let parsed: serde_json::Value = serde_json::from_str(&body)
                                .unwrap_or(serde_json::Value::Null);
                            // OpenAI-compatible: {"data":[{"id":...}]}. Some
                            // servers answer {"models":[...]} or a bare array.
                            // OpenAI-compatible: {"data":[{"id":...}]}. Some
                            // servers answer {"models":[...]} or a bare array.
                            let mut entries: Vec<serde_json::Value> = Vec::new();
                            if let Some(arr) = parsed.get("data").and_then(|d| d.as_array()) {
                                entries.extend(arr.iter().cloned());
                            } else if let Some(arr) = parsed.get("models").and_then(|d| d.as_array()) {
                                entries.extend(arr.iter().cloned());
                            } else if let Some(arr) = parsed.as_array() {
                                entries.extend(arr.iter().cloned());
                            }
                            // Classify: an embedding model has no completion/output
                            // pricing (and no context window); chat models price
                            // completion tokens. The name is the fallback signal.
                            let is_embedding = |v: &serde_json::Value| -> bool {
                                let id = v
                                    .as_str()
                                    .map(str::to_string)
                                    .or_else(|| v.get("id").and_then(|i| i.as_str()).map(str::to_string))
                                    .or_else(|| v.get("name").and_then(|i| i.as_str()).map(str::to_string))
                                    .unwrap_or_default();
                                let lower = id.to_lowercase();
                                // Name only. The metadata a gateway returns is not
                                // a reliable signal: this endpoint reports
                                // `context: 0` and per-request-only pricing for
                                // plenty of chat models (GLM-5.3, gemini-flash-high,
                                // qwen-image-max, grok-voice), so pricing would put
                                // ~90 chat models in the embedding list.
                                lower.contains("embed")
                                    || lower.contains("bge-")
                                    || lower.contains("gte-")
                                    || lower.starts_with("e5-")
                                    || lower.contains("retrieval")
                            };
                            let mut chat: Vec<String> = Vec::new();
                            let mut embedding: Vec<String> = Vec::new();
                            for v in &entries {
                                let id = v
                                    .as_str()
                                    .map(str::to_string)
                                    .or_else(|| v.get("id").and_then(|i| i.as_str()).map(str::to_string))
                                    .or_else(|| v.get("name").and_then(|i| i.as_str()).map(str::to_string))
                                    .unwrap_or_default();
                                if id.is_empty() {
                                    continue;
                                }
                                if is_embedding(v) {
                                    embedding.push(id);
                                } else {
                                    chat.push(id);
                                }
                            }
                            chat.sort();
                            chat.dedup();
                            embedding.sort();
                            embedding.dedup();
                            let selected: Vec<String> = match kind.as_str() {
                                "embedding" | "embeddings" => embedding.clone(),
                                "all" => {
                                    let mut all = chat.clone();
                                    all.extend(embedding.clone());
                                    all
                                }
                                _ => chat.clone(),
                            };
                            if selected.is_empty() {
                                eprintln!("no {} models found at {url} (response: {})", kind, &body.chars().take(200).collect::<String>());
                                std::process::exit(1);
                            }
                            if cli.json {
                                println!(
                                    "{}",
                                    serde_json::to_string_pretty(&serde_json::json!({
                                        "url": url,
                                        "models": selected,
                                        "chat_models": chat,
                                        "embedding_models": embedding,
                                    }))?
                                );
                            } else {
                                for id in &selected {
                                    println!("{id}");
                                }
                            }
                        }
                        Err(ureq::Error::StatusCode(code)) => {
                            eprintln!("{url} returned HTTP {code}");
                            std::process::exit(1);
                        }
                        Err(e) => {
                            eprintln!("{url} unreachable: {e}");
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
        Command::Doctor => {
            imc_cli::doctor::run_doctor(&cfg, &root)?;
        }
        Command::Mcp { action } => match action {
            None => imc_cli::mcp::serve(&root)?,
            Some(McpAction::Install { config }) => {
                let path = imc_cli::mcp::install(&root, config.clone())?;
                println!("wrote {}", path.display());
                println!("MCP client will discover: search, ask, trace, read, context, memory_search, pack");
            }
        },
        Command::Start => {
            // Bare `imc`: the whole setup in one command — the proxy that
            // Claude Code talks to, and the dashboard to drive everything else.
            let proxy_port: u16 = 12345;
            let web_port: u16 = 4789;
            if cli.json {
                let proxy = match imc_cli::claude::start_proxy(&cfg, proxy_port) {
                    Ok(v) => v,
                    Err(e) => serde_json::json!({"error": e.to_string()}),
                };
                let running = imc_cli::claude::proxy_status(&cfg)
                    .map(|s| s.get("running").and_then(|v| v.as_bool()).unwrap_or(false))
                    .unwrap_or(false);
                println!(
                    "{}",
                    serde_json::json!({
                        "proxy": {"port": proxy_port, "running": running, "state": proxy},
                        "dashboard": {"port": web_port, "url": format!("http://localhost:{web_port}")},
                    })
                );
            } else {
                match imc_cli::claude::start_proxy(&cfg, proxy_port) {
                    Ok(v) => {
                        let pid = v.get("pid").and_then(|p| p.as_i64()).unwrap_or(0);
                        println!("proxy    → http://127.0.0.1:{proxy_port}  (pid {pid})");
                    }
                    Err(e) => {
                        println!("proxy    → not started: {e}");
                        println!("           the proxy runs as `imc proxy serve` — nothing else to build");
                    }
                }
                wire_claude_to_proxy(&cfg, &root, proxy_port);
                imc_cli::web::run(web_port, false, None)?;
            }
        }
        Command::Install {
            bin_dir,
            no_dashboard,
        } => {
            for line in imc_cli::install::run(bin_dir.clone(), !*no_dashboard)? {
                println!("{line}");
            }
        }
        Command::Update { check, force } => {
            use imc_cli::update;
            let timeout = std::time::Duration::from_secs(if *check { 5 } else { 60 });
            match update::refresh(timeout) {
                Ok(latest) => {
                    let newer = update::is_newer(&latest, update::current_version());
                    if *check {
                        // Say whether the release can actually be installed
                        // here: a release without this platform's asset is a
                        // dead end for the user, and "an update exists" alone
                        // would send them into it.
                        let release = update::fetch_release(timeout).ok();
                        let assets = release.as_ref().map(|r| r.assets.clone()).unwrap_or_default();
                        let installable = release
                            .as_ref()
                            .map(|r| r.has_asset_for_this_platform())
                            .unwrap_or(false);
                        let platform = update::platform();
                        if cli.json {
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&serde_json::json!({
                                    "current": update::current_version(),
                                    "latest": latest,
                                    "update_available": newer,
                                    "platform": platform,
                                    "assets": assets,
                                    "installable": installable,
                                }))?
                            );
                        } else if newer && !installable {
                            println!(
                                "update available: v{} → v{} — but the release has no asset for {}",
                                update::current_version(),
                                latest,
                                platform.unwrap_or("this platform")
                            );
                            if !assets.is_empty() {
                                println!("assets in the release: {}", assets.join(", "));
                            }
                        } else if newer {
                            println!(
                                "update available: v{} → v{}",
                                update::current_version(),
                                latest
                            );
                        } else {
                            println!("imc v{} is up to date", update::current_version());
                        }
                    } else if newer || *force {
                        match update::install_with_force(timeout, *force) {
                            Ok((tag, path)) => println!(
                                "updated v{} → v{} ({})",
                                update::current_version(),
                                tag,
                                path.display()
                            ),
                            Err(e) => eprintln!("update failed: {e:#}"),
                        }
                    } else {
                        println!("imc v{} is up to date", update::current_version());
                    }
                }
                Err(e) => {
                    eprintln!("could not check for updates: {e:#}");
                    if !*check {
                        std::process::exit(1);
                    }
                }
            }
        }
        Command::Capture { action } => {
            use imc_core::capture;
            match action {
                None | Some(CaptureAction::Status) => {
                    let status = capture::status();
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&status)?);
                    } else {
                        println!(
                            "prompt capture: {}",
                            if status["enabled"].as_bool().unwrap_or(false) { "on" } else { "off" }
                        );
                        println!(
                            "  stored: {} payload(s) in {}",
                            status["captures"].as_u64().unwrap_or(0),
                            status["dir"].as_str().unwrap_or("")
                        );
                    }
                }
                Some(CaptureAction::On) => {
                    capture::set_enabled(true)?;
                    println!("prompt capture: on");
                }
                Some(CaptureAction::Off) => {
                    capture::set_enabled(false)?;
                    println!("prompt capture: off");
                }
                Some(CaptureAction::List) => {
                    let list = capture::list_captures();
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&list)?);
                    } else {
                        for c in &list {
                            println!(
                                "{}\t{}\t{} B\t{}",
                                c.id,
                                c.model,
                                c.size,
                                c.preview
                            );
                        }
                        if list.is_empty() {
                            println!("no captures yet");
                        }
                    }
                }
                Some(CaptureAction::Show { id }) => {
                    match capture::get_capture(&id) {
                        Some(v) => println!("{}", serde_json::to_string_pretty(&v)?),
                        None => bail!("no capture with id {id:?}"),
                    }
                }
                Some(CaptureAction::Rm { id }) => {
                    if capture::delete_capture(&id) {
                        println!("deleted {id}");
                    } else {
                        bail!("no capture with id {id:?}");
                    }
                }
                Some(CaptureAction::Clear) => {
                    let n = capture::clear_captures();
                    println!("deleted {n} capture(s)");
                }
            }
        }
        Command::Web { port, no_open, url } => {
            imc_cli::web::run(*port, *no_open, url.clone())?;
        }
    }


    // Tell the user when a newer release exists. The check itself is cached
    // (<data dir>/update_check.json) and refreshed by a detached background
    // process, so the hot path is a file read, not a network call.
    if !cli.json
        && !matches!(
            &cli.command,
            Command::Update { .. } | Command::Proxy { .. }
        )
    {
        imc_cli::update::maybe_notify();
    }
    Ok(())
}

/// Point Claude Code at the proxy, and make sure the model it asks for
/// exists upstream.
///
/// `imc setup` writes `.claude/settings.local.json` with
/// `ANTHROPIC_BASE_URL` pointing at the proxy; without it Claude Code talks to
/// the API directly and the proxy never sees a request. The placeholder model
/// name it uses (`claude-pro-max`) is not a model any upstream serves, so a
/// mapping to the configured model is created as well — otherwise the proxy
/// forwards a name the API rejects.
fn wire_claude_to_proxy(cfg: &Config, root: &Path, port: u16) {
    let settings = root.join(".claude/settings.local.json");
    let wired = std::fs::read_to_string(&settings)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| {
            v.get("env")
                .and_then(|e| e.get("ANTHROPIC_BASE_URL"))
                .and_then(|u| u.as_str())
                .map(|u| u.contains("127.0.0.1"))
        })
        .unwrap_or(false);
    if !wired {
        match imc_cli::claude::write_claude_settings(root, port) {
            Ok(p) => {
                println!("wiring   → {} (Claude Code now goes through the proxy)", p.display());
                println!("           restart Claude Code for this to take effect");
            }
            Err(e) => println!("wiring   → skipped: {e}"),
        }
    }

    // The placeholder model needs a mapping to a model the endpoint serves.
    let target = cfg.llm_model();
    if target.is_empty() {
        return;
    }
    let placeholder = "claude-pro-max";
    if target == placeholder {
        return;
    }
    use imc_proxy::imc_proxy_types::config_store as cs;
    let mut proxy_cfg = cs::read_config_from_disk();
    let mut mappings = proxy_cfg.model_mappings.clone().unwrap_or_default();
    if !mappings.contains_key(placeholder) {
        mappings.insert(placeholder.to_string(), target.clone());
        proxy_cfg.model_mappings = Some(mappings);
        if cs::write_config_to_disk(&proxy_cfg).is_ok() {
            println!("mapping  → {placeholder} → {target}");
        }
    }
}

/// Resolve a user-typed symbol to the stored definitions.
/// Accepts `name`, `file::qualname` (used verbatim), or `name` + `--file`.
fn resolve_symbols(
    store: &store::Store,
    symbol: &str,
    file: Option<&str>,
) -> Result<Vec<imc_core::types::Symbol>> {
    // `file::qualname` is an explicit id — take it as-is.
    if symbol.contains("::") {
        let mut exact = store.find_symbols_by_name(symbol, 25);
        exact.retain(|s| s.id == symbol || s.qualname == symbol);
        if !exact.is_empty() {
            return Ok(exact);
        }
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
        // Names like `search` or `build_index` are common; show every
        // definition instead of forcing the caller to disambiguate.
        _ => Ok(cands),
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

static STOP_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(unix)]
fn setup_signal_handler() {
    extern "C" fn handler(_: libc::c_int) {
        STOP_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    unsafe {
        libc::signal(libc::SIGINT, handler as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, handler as *const () as libc::sighandler_t);
    }
}

#[cfg(not(unix))]
fn setup_signal_handler() {}