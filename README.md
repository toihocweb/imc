# IndexMyCode — Rust port

Code intelligence for coding agents: **AST index + lexical/vector retrieval + call graph**,
exposed as a single CLI that Claude Code (or any agent) drives directly.

No UI. No Python runtime. No vector server. One binary, one SQLite file per project.

```
imc claude setup      # ← the one command you need: index + wire Claude Code + start proxy
imc search "..."      # hybrid search, no LLM cost
imc trace <symbol>    # what it calls / what calls it (blast radius)
imc read <file>       # token-optimised reader (replaces Read for big files)
```

---

## Why it looks like this

Three ideas drive the design:

1. **Search must not cost tokens.** Retrieval is FTS5 BM25 + call-graph expansion —
   pure local computation, zero LLM calls. The model is only needed for the *answer*,
   never for the *lookup*.
2. **Prefer structure over prose.** Every chunk is an AST symbol, every hit carries
   `file:line` and the symbol it belongs to, and every symbol has caller/callee edges.
3. **Degrade, don't die.** Missing embedding key ⇒ the vector leg switches off and search
   keeps working on BM25 + graph. No service process to babysit.

---

## Install & build

```bash
# Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable

cd imc-rs
cargo build --release
# binaries: target/release/imc  (CLI)  +  target/release/imc-proxyd (proxy daemon)

cargo test --release      # 133 tests, 0 failures
```

Requires a C compiler — tree-sitter grammars and the bundled SQLite build from source.

---

## Commands

| Command | What it does |
|---|---|
| `imc index [path] [-i <glob>] [--force]` | Incremental index. Unchanged files skipped by MD5. |
| `imc search "<query>"` | Hybrid search. `query \| directive` splits facets; `-l N` overrides; `--no-vector` forces BM25 + graph. |
| `imc trace <symbol> -d callees\|callers [--depth N] [--all] [-f file]` | Call tree / blast radius. Merges every definition when a name is ambiguous. |
| `imc read <file> [--symbols] [--method NAME] [--offset N] [--limit N] [--raw]` | Compressed reader: blank-line strip, JSON smart-strip, auto-truncate with summary header. |
| `imc context <target>` | One-shot resolved context for a file/module (outline, deps, tests). |
| `imc claude setup\|wire\|start\|stop\|status` | Claude Code integration (see below). |
| `imc config get\|set\|list\|path\|test\|migrate` | Edit `~/.imc_config.yaml`: LLM url/key/model, embedding model. |
| `imc proxy serve\|status\|stop` | The LLM proxy Claude Code talks to (daemon binary `imc-proxyd`). |
| `imc graph <symbol>` | Quick definitions / callees / callers dump. |
| `imc memory-add/-search/-list/-delete`, `imc learn <file>` | Project memory (rules, decisions). |
| `imc stats`, `imc projects` | Index statistics, registered projects. |

Global flags: `-C/--project <dir>` (defaults to the active project) and `--json`
(every command emits machine-readable JSON).

---

## Claude Code integration

```bash
imc claude setup
```

One command that:

1. indexes the current project (incremental, `--force` to redo),
2. writes `.claude/settings.local.json` — points `ANTHROPIC_BASE_URL` at
   `http://127.0.0.1:12345` (our proxy) and pins model env vars, **preserving** any env
   you already had,
3. writes/updates an `imc` block in the project's `CLAUDE.md` — idempotent via
   `<!-- imc:begin -->` / `<!-- imc:end -->` markers, so re-running never duplicates it,
4. registers the project and starts the proxy daemon (`pid`/`port` in
   `$IMC_DATA_DIR/proxy_state.json`, log in `$IMC_DATA_DIR/logs/proxy.log`),
5. prints `✓` per step, then tells you to just run `claude`.

`imc claude wire` does only steps 2–3 (no indexing). `imc claude stop` kills the daemon.

The `CLAUDE.md` block teaches the agent when to reach for which command — search first,
`imc read` for big files, `imc trace` before any rename/delete, `imc memory-search` for
repo conventions.

---

## Search quality — measured, not claimed

Harness: 12 realistic questions about the original `imc` Python repo (183 indexed files,
1 070 symbols, 6 609 edges), scored on whether the expected file lands in the top-k.

| Configuration | top-1 | top-5 |
|---|---|---|
| BM25 (broken prefix syntax) + offline hashing vector | 1/12 (8%) | 7/12 (58%) |
| Fixed BM25 + offline hashing vector | 3/12 (25%) | 8/12 (67%) |
| **Fixed BM25 + graph only (default, no embedding key)** | **5/12 (42%)** | **8/12 (67%)** |

Two findings that shaped the code:

- **A silently dead BM25 leg.** The FTS5 prefix token must be written `"tok"*` — the star
  *outside* the quotes. Writing `"tok*"` is a literal token that matches nothing, so the
  whole lexical retriever returned zero rows while the vector leg papered over it. It
  looked like ranking noise instead of an outage. `imc search --no-vector` is the canary.
- **The offline hashing embedder hurt.** It is a lexical stand-in, not semantics: it
  dragged top-1 from 42% down to 25%. It is therefore **opt-in** (`IMC_VECTOR=1`), and
  the vector leg switches on only when a real embedding endpoint is configured.

Retrieval pipeline:

1. **Lexical** — SQLite FTS5 `bm25()`: a strict AND pass plus a wider OR pass
   (OR-only hits carry a 0.55 penalty), stopwords removed, last token prefix-matched.
2. **Graph** — per-hit evidence: sibling definitions in the same file, direct callees,
   direct callers.
3. **Vector (optional)** — cosine over f32 blobs when `IMC_EMBED_API_KEY`/`OPENAI_API_KEY`
   is set; fused with the lexical list via Reciprocal Rank Fusion (k=60).
4. **Rerank** — filename/path/symbol-name boosts, sort, then per-file diversity
   (max 2 chunks per file) so one big file cannot monopolise the context.

Reranking happens *before* top-K selection — boosts computed after truncation do nothing.

---

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
(WAL, `synchronous=NORMAL`). Measured on the 183-file `imc` repo: **~2.0 s** full,
**~0.2 s** when nothing changed.

**Languages** (tree-sitter grammars): Python, JavaScript, TypeScript, TSX, Rust, Go.
Markdown/JSON/YAML/TOML and friends are indexed as text windows.

---

## Status

| Crate | State |
|---|---|
| `imc-core`, `imc-parser`, `imc-index`, `imc-search`, `imc-embed` | done, tested |
| `imc-trace`, `imc-find`, `imc-support`, `imc-memory`, `imc-cli` | done, tested |
| `imc-proxy-types`, `imc-proxy-copilot` | done (wire types + Anthropic⇄Responses⇄ChatCompletions) |
| `imc-proxy` (routes, upstream providers, memory injection) | port in progress — `imc claude setup` wires the config, the daemon still needs this |

Verified on the original `imc` Python repo: 133 tests pass, 183 files → 1 070 symbols /
6 609 edges, full index ~2.0 s, re-index ~0.2 s.

---

## Porting map (Python → Rust)

| Python | Rust crate |
|---|---|
| `main/ingest/parser.py`, `main/indexing/graph.py` | `imc-parser` |
| `main/indexing/semantic.py` | `imc-index`, `imc-embed` |
| `main/search/{semantic,graph,context,limits}.py` | `imc-search`, `imc-support` |
| `main/core/tracer.py` | `imc-trace` |
| `main/cli/commands/{find,context}.py` | `imc-find` |
| `main/core/prompts.py`, `main/core/endpoint_bridge.py` | `imc-support` |
| `main/memory/*` | `imc-memory` |
| `main/project/manager.py`, `main/core/config.py` | `imc-core` |
| `main/proxy/**` | `imc-proxy`, `imc-proxy-types`, `imc-proxy-copilot` (in progress) |
| `main/api/**`, `ui/`, `desktop/` | dropped by design (no UI) |

Python: 23 070 lines under `main/`. Rust: ~26 600 lines across 14 crates, of which the
agent-facing core (index + search + trace + read + context) is ~7 000.

### Deliberately not ported

- `main/api/**` (FastAPI + git/jira/copilot routers) and `ui/`/`desktop/` — the CLI reads
  the store directly; the HTTP surface only existed to serve the UI.
- The MCP server — Claude Code integrates via CLI + hooks instead.
- LLM-backed `/api/ask` summarisation — that work belongs to the agent, not the indexer.

---

## Configuration

`~/.imc_config.yaml` (same file the Python version reads; `IMC_CONFIG` overrides the path;
`imc config migrate` copies an old `~/.indexer_config.yaml`):

```yaml
llm:
  base_url: http://127.0.0.1:12345      # upstream for the proxy / LLM calls
  api_key: sk-...
  model: claude-pro-max
embedding:
  model: text-embedding-3-small
ingest:
  ignored_dirs: [vendor, third_party]
```

```bash
imc config set llm.base_url https://api.anthropic.com
imc config set llm.api_key sk-ant-...
imc config test          # POST {base_url}/embeddings and report
imc config list          # secrets masked
```

| Env | Default | Purpose |
|---|---|---|
| `IMC_CONFIG` | `~/.imc_config.yaml` | Config file path |
| `IMC_DATA_DIR` | `~/.indexmycode-rs` | Registry + per-project DBs |
| `IMC_EMBED_API_KEY` / `OPENAI_API_KEY` | unset | Enables the vector leg |
| `IMC_EMBED_BASE_URL` | from `llm.base_url` | Embeddings endpoint |
| `IMC_EMBED_MODEL` | `text-embedding-3-small` | Embedding model |
| `IMC_VECTOR` | unset | `1` forces the offline hashing embedder on |
| `IMC_MAX_FILE_BYTES` | `1048576` | Skip files larger than this |

Per-project DB: `$IMC_DATA_DIR/projects/<md5(path)[:16]>/index.db`.

---

## License

MIT (unchanged from the Python project).