# IndexMyCode — imc (Rust)

Code intelligence for coding agents: **AST index + lexical/vector retrieval + call graph**,
exposed as a single CLI that Claude Code (or any agent) drives directly.

No UI. No Python runtime. No vector server. One binary, one SQLite file per project.

```bash
imc claude setup      # ← the one command you need: index + wire Claude Code + start proxy
imc search "..."      # answer from the index with the LLM (-r for raw hits)
imc trace <symbol>    # what it calls / what calls it (blast radius)
imc read <file>       # token-optimised reader (replaces Read for big files)
```

## Install

**One-liner:**

```bash
curl -fsSL https://raw.githubusercontent.com/toihocweb/imc/master/install.sh | bash
imc .
```

**From source:**

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
git clone https://github.com/toihocweb/imc.git
cd imc
cargo build --release
cargo test --release        # 325 tests, 0 failures
binaries: target/release/imc  (CLI)
```

Requires a C compiler — tree-sitter grammars and the bundled SQLite build from source.

## First run

```bash
# 1. Configure LLM (needed for imc search answers; optional for raw hits)
imc config set llm.base_url https://api.xah.io/v1
imc config set llm.api_key  sk-...
imc config set llm.model    claude-haiku-4.5   # cheap model, good for imc search

# 2. Index the current project
imc .

# 3. Use it
imc search "authentication flow"
imc trace AuthController.login --callers
imc context main/api/routers/git.py
```

## Commands

| Command | What it does |
|---|---|
| `imc [path]` / `imc index [path] [-i <glob>] [--force]` | Incremental index. Unchanged files skipped by MD5. `imc .` is a shortcut for `imc index .`. |
| `imc search "<query>"` | Retrieve (BM25 + vector + graph boosts), then answer with the LLM. `-r/--raw` prints ranked hits only, `--json` emits hits, `-l N`/`-n N` override count, `--no-vector` forces BM25. |
| `imc trace <target> [-d N] [-r/--callers] [-b/--both] [-a/--all] [-f file]` | Call tree / blast radius, also for `GET /api/path` route targets. `imc callers` / `callees` / `blast` are shorthands. |
| `imc read <file> [--symbols] [--method NAME] [--offset N] [--limit N] [--raw]` | Token-optimised reader: blank-line strip, JSON smart-strip, auto-truncate with summary header. |
| `imc context <target>` | One-shot resolved context for a file/module (outline, deps, tests). |
| `imc claude setup\|wire\|start\|stop\|status` | Claude Code integration (see below). |
| `imc config get\|set\|list\|path\|test\|migrate\|setup` | Edit `~/.imc_config.yaml`: LLM url/key/model, embedding model. `setup` walks you through interactively. |
| `imc proxy serve\|status\|stop` | The LLM proxy Claude Code talks to. |
| `imc graph <symbol>` | Quick definitions / callees / callers dump. |
| `imc memory-add/-search/-list/-delete`, `imc learn <file>` | Project memory (rules, decisions). |
| `imc stats`, `imc list` | Index statistics, registered projects (files, symbols, edges, chunks, memories, DB size). |

Global flags: `-C/--root <dir>` (defaults to the active project) and `--json`
(every command emits machine-readable JSON). `search`/`trace`/`context` also take
`-p/--project <registered-id-or-name>`.

## Claude Code integration

```bash
imc claude setup
```

One command that:

1. indexes the current project (incremental, `--force` to redo),
2. writes `.claude/settings.local.json` — points `ANTHROPIC_BASE_URL` at
   `http://127.0.0.1:12345` (our proxy) and pins model env vars, **preserving**
   any env you already had,
3. writes `.claude/rules/imc-rule.md` — project-local rules teaching the agent
   when to use each `imc` command,
4. registers the project and starts the proxy daemon,
5. prints `✓` per step, then tells you to just run `claude`.

`imc claude wire` does only steps 2–3 (no indexing). `imc claude stop` kills the daemon.

## Config

`~/.imc_config.yaml` (`IMC_CONFIG` overrides the path; `imc config migrate` copies
an old `~/.indexer_config.yaml`):

```yaml
llm:
  base_url: http://127.0.0.1:12345      # upstream for the proxy / LLM calls
  api_key: sk-...
  model: claude-haiku-4.5
embedding:
  model: text-embedding-3-small
ingest:
  ignored_dirs: [vendor, third_party]
```

```bash
imc config set llm.base_url https://api.xah.io/v1
imc config set llm.api_key sk-ant-...
imc config test          # POST {base_url}/embeddings and report
imc config list          # secrets masked
imc config setup         # interactive walkthrough
```

| Env | Default | Purpose |
|---|---|---|
| `IMC_CONFIG` | `~/.imc_config.yaml` | Config file path |
| `IMC_DATA_DIR` | `~/.indexmycode-rs` | Registry + per-project DBs |
| `IMC_EMBED_API_KEY` / `OPENAI_API_KEY` | unset | Enables the vector leg |
| `IMC_EMBED_BASE_URL` | from `llm.base_url` | Embeddings endpoint |
| `IMC_EMBED_MODEL` | `text-embedding-3-small` | Embedding model |
| `IMC_MAX_FILE_BYTES` | `1048576` | Skip files larger than this |

Per-project DB: `$IMC_DATA_DIR/projects/<md5(path)[:16]>/index.db`.

## Architecture

```
crates/
  imc-core        config, ~/.imc_config.yaml, gitignore-aware walk, MD5, SQLite store
  imc-parser      tree-sitter AST: symbols, call/import edges, AST-scoped chunks
  imc-index       walk → diff → parallel parse → chunk → embed → store → FTS
  imc-search      BM25 + graph + vector, RRF, rerank
  imc-embed       embedding providers (OpenAI-compatible / opt-in hashing / none)
  imc-trace       call-hierarchy tracer (callees, callers, blast radius, builtin filter)
  imc-find        `imc read` + `imc context`
  imc-support     prompt library, endpoint bridge, context packing, search-limit policy
  imc-memory      project memory: CRUD, dedup, retrieval, extraction
  imc-cli         the `imc` binary
  imc-proxy       transparent LLM proxy: routes, upstream providers, context injection
  imc-proxy-types wire types + provider config (Anthropic / OpenAI / Copilot)
  imc-proxy-copilot  Anthropic ⇄ Responses ⇄ Chat Completions translation
```

**Storage.** One SQLite file per project: `files`, `symbols`, `edges`, `chunks`,
`embeddings`, an FTS5 index, plus the project registry. No LanceDB, no NetworkX,
no llama-index.

**Indexing.** Walk (gitignore + binary + hashed-bundle filters) → MD5 diff against
`files` → tree-sitter parse **on rayon threads** → single-threaded SQLite writes
(WAL, `synchronous=NORMAL`).

**Languages** (tree-sitter grammars): Python, JavaScript, TypeScript, TSX, Rust, Go.
Markdown/JSON/YAML/TOML and friends are indexed as text windows.

## License

MIT.
