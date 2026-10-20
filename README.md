# imc

**Code intelligence for coding agents.** `imc` indexes a repository with tree-sitter, keeps a
call graph and a lexical/vector index over it, and answers the questions an agent actually
asks — *"where is this handled?"*, *"what breaks if I change it?"*, *"what did we decide about
it?"*.

One Rust binary. No Python runtime. No vector database. One SQLite file per project.

```bash
imc search "authentication flow"     # answer with path:line citations
imc trace AuthController --callers    # blast radius before you rename
imc pack "how does login work"        # one token-budgeted context bundle
```

---

## Contents

- [Install](#install)
- [Quick start](#quick-start)
- [Capabilities](#capabilities)
  - [Search](#search)
  - [Call graph](#call-graph)
  - [Context packing](#context-packing)
  - [Memory](#memory)
  - [MCP server](#mcp-server)
  - [Web dashboard](#web-dashboard)
  - [LLM proxy and compression](#llm-proxy-and-compression)
  - [Prompt capture](#prompt-capture)
- [Configuration](#configuration)
- [Command reference](#command-reference)
- [Benchmarks](#benchmarks)
- [Architecture](#architecture)
- [Development](#development)
- [License](#license)

---

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/toihocweb/imc/master/install.sh | bash
imc .
```

From source:

```bash
git clone https://github.com/toihocweb/imc.git
cd imc
cargo build --release
./target/release/imc --help
```

Rust 1.80+ and a C compiler to build (tree-sitter grammars and the bundled SQLite are
compiled from source). The prebuilt installer needs neither.

---

## Quick start

```bash
# 1. point it at a model endpoint — optional, raw hits work without it
imc config setup

# 2. index the project (incremental; --force rebuilds)
imc .

# 3. use it
imc search "how are sessions invalidated"
imc trace SessionStore.rotate --callers
```

For Claude Code, one command wires everything — index, `.claude/settings.local.json`, the
agent rule file, `.mcp.json`, and the proxy:

```bash
imc claude setup
claude
```

`imc claude` starts the proxy first, then the `claude` binary detached, so the session
survives closing the terminal.

---

## Capabilities

### Search

Hybrid retrieval: BM25 (SQLite FTS5) + optional vector similarity, fused with RRF, plus
graph expansion around symbols.

```bash
imc search "tax calculation"              # LLM answer with citations
imc search "tax calculation" --raw        # ranked chunks only, no LLM
imc search "exact symbol name"            # symbol lookup
imc search "retry policy" --no-vector     # BM25 + graph only
```

Retrieval has a relevance floor: a junk query returns nothing rather than padding the list
with plausible-looking noise. A failed search is recorded as project memory automatically
so the next session can surface it — opt out with `--no-on-miss`.

### Call graph

```bash
imc trace AuthController.login           # what it calls
imc trace AuthController.login --callers  # who calls it (blast radius)
imc trace "GET /api/login" --both        # HTTP route → handler, both directions
imc trace SessionStore --depth 3         # deeper tree
```

HTTP routes are first-class trace targets: `imc trace "GET /api/users"` resolves to the
handler and the component behind it.

### Context packing

`imc pack` assembles everything an agent needs for a task — retrieval, the graph
neighbourhood, and project memories — into a single bundle that fits a token budget.
Shortened items carry a handle you can expand later.

```bash
imc pack "why is login slow" --budget 6000
imc retrieve <handle>          # the original behind a shortened item
```

### Memory

Durable, per-project knowledge: conventions, decisions, business rules.

```bash
imc memory-add "changelog dates are ISO 8601" --type business_rule
imc memory-search "changelog"
imc memory-list
imc learn transcript.md        # extract memories from a transcript
```

An empty `memory-search` result means *no matching memory*, not *this project has no
memories*.

### MCP server

Expose imc to any MCP client (Claude Code, Cursor, Cline, …) as real tools with schemas —
no shell quoting, no guessing flags.

```bash
imc mcp install     # writes .mcp.json for this project
imc mcp             # serve over stdio
```

Tools: `search`, `ask`, `trace`, `read`, `context`, `memory_search`, `pack`. Every call is
pinned to the project's root, so a tool call can never answer for a different repository.

### Web dashboard

A shadcn/ui dashboard over the same CLI — browse projects, search, trace, inspect memories,
watch the compression store, capture prompts, and edit the endpoint configuration.

```bash
imc web                          # opens http://localhost:4789, starting it if needed
imc web --url http://<host>:4789 # remote host
```

Pages: overview, projects, per-project detail (search + trace + memory tabs), search, trace,
memory, compression, captures, settings. Light and dark themes, responsive down to phone
width.

### LLM proxy and compression

The proxy sits between the agent and the model, so it sees every tool result about to be
re-read. Large ones are compressed on the way through and the original is kept in a CCR
store — the model gets a handle and can pull the full text back verbatim.

| Kind | What it does |
|---|---|
| `json_array` | Collapses runs of identical items, elides a field that duplicates a sibling, windows fields over 1.5 KB to head+tail. Short arrays of distinct items stay intact. |
| `build_log` | Keeps every `ERROR` / `WARN` / failure line and the diagnostic gutters; drops routine progress output. |
| `source_code` | String-literal-aware comment stripping (a `//` inside `"..."` is code), block-level repeat collapse, blank-line runs. |
| `git_diff` | Dedupes identical file sections, elides the commit-message body, drops `index <hash>` lines, trims long unchanged runs. **Every changed line is preserved.** |
| `plain_text` | Drops redundant sentences, keeps the first, the last, and the keyword-dense middle. |

Two invariants hold across all of them: **the output is never larger than the input**, and
every elision is announced in place with a count.

`/v1/messages` compresses tool results before forwarding. A result is only touched when it
is over 400 tokens **and** the compressor saves at least 20%; `is_error` results are never
touched, because the error text is the answer.

```
POST /v1/compress   {"content": "..."}   -> {id, kind, compressed, tokens, ratio}
POST /v1/retrieve   {"id": "..."}        -> {id, original, compressed}
GET  /v1/stats                            -> totals + overall ratio
```

The store is `<data dir>/proxy_ccr.db` (`IMC_CCR_DB` overrides), and the same store is
reachable from the terminal — a handle in a log or transcript is actionable:

```bash
imc compress --text "$(cat build.log)" --store   # -> id 3139ac0529...
imc retrieve 3139ac052928fd49                    # exact original bytes
imc ccr stats                                    # totals + overall ratio
imc ccr prune --days 30                          # expire old handles
```

### Prompt capture

Turn it on and every payload the proxy sends upstream is written verbatim — system prompt,
tools, injected memories, tool results. The fastest way to answer *"what did the agent
actually send?"*.

```bash
imc capture on
imc capture list
imc capture show system12
```

Off by default; toggleable from the dashboard at `/captures`.

---

## Configuration

`~/.imc_config.yaml` holds one endpoint for everything:

```yaml
llm:
  base_url: https://api.example.com/v1
  api_key: sk-...
  model: claude-haiku-4.5
embedding:
  model: text-embedding-3-small
ingest:
  ignored_dirs: [vendor, third_party]
```

```bash
imc config setup        # interactive
imc config list        # secrets masked
imc config test        # probe the endpoint
imc config models      # what the endpoint serves
imc doctor             # diagnose everything, print the fix commands
```

`imc doctor` reports the things that actually break setups — a missing `llm.model` (the
built-in fallback is endpoint-specific and usually 404s), an unreachable endpoint, a project
that was never indexed, a proxy that is not running.

| Env | Default | Purpose |
|---|---|---|
| `IMC_CONFIG` | `~/.imc_config.yaml` | Config file path |
| `IMC_DATA_DIR` | `~/.indexmycode-rs` | Registry + per-project databases |
| `IMC_EMBED_API_KEY` / `OPENAI_API_KEY` | unset | Enables the vector leg |
| `IMC_EMBED_BASE_URL` | from `llm.base_url` | Embeddings endpoint |
| `IMC_EMBED_MODEL` | `text-embedding-3-small` | Embedding model |
| `IMC_MAX_FILE_BYTES` | `1048576` | Skip files larger than this |
| `IMC_CCR_DB` | `<data dir>/proxy_ccr.db` | Compression store (originals kept for retrieval) |

Per-project database: `$IMC_DATA_DIR/projects/<md5(path)[:16]>/index.db`.

---

## Command reference

| Command | What it does |
|---|---|
| `imc [path]` / `imc index [path]` | Incremental index (`--force` rebuilds, `-i` adds ignore globs). `imc .` is a shortcut for `imc index .`. |
| `imc search "<query>"` | Retrieve (BM25 + vector + graph), then answer with the LLM. `--raw` for hits only, `--json` for machine output, `-l N` for the limit, `--no-vector` to force BM25. |
| `imc trace <target>` | Call tree / blast radius, also for `GET /api/path` route targets. `--callers`, `--callees`, `--both`, `-d N`, `-f file`. |
| `imc graph <symbol>` | Definitions, callees and callers of a symbol. |
| `imc read <file>` | Token-optimised reader: blank-line strip, JSON smart-strip, auto-truncate with a summary header. `--symbols`, `--method`, `--offset`, `-l`, `--raw`. |
| `imc context <target>` | Resolved context for a file/module: outline, deps, tests. |
| `imc pack "<query>"` | One token-budgeted bundle of context: retrieval + graph neighbourhood + project memories. `--budget`, `--candidates`, `--no-vector`, `--no-memories`, `--graph`. |
| `imc compress [--text T]` | Compress content by detected kind. Reads stdin when `--text` is absent; `--raw` prints only the text, `--store` keeps the original and prints its id. |
| `imc retrieve <id>` | The original behind a compression handle (`--compressed` for the short form). |
| `imc ccr stats\|list\|clear\|prune\|tune` | Inspect the compression store. `tune --apply` adjusts the savings threshold; `tune --auto` re-tunes in the background. |
| `imc memory-add/-search/-list/-delete/-prune` | Project memory. `--type`, `--scene`, `--category`, `--importance`, `--path`, `--tag`. |
| `imc learn <file>` | Extract memories from a transcript or markdown file. |
| `imc claude setup\|wire\|start\|stop\|status` | Claude Code integration. |
| `imc proxy serve\|status\|stop` | The LLM proxy. |
| `imc mcp [install]` | MCP server; `install` writes `.mcp.json` for the project. |
| `imc capture status\|on\|off\|list\|show\|rm\|clear` | Capture the prompts the proxy sends upstream. |
| `imc config get\|set\|list\|path\|test\|models\|migrate\|setup` | View and edit the configuration. |
| `imc doctor` | Diagnose the environment and print fix commands. |
| `imc web` | Open the dashboard, starting it if needed. |
| `imc list` | Registered projects with counts and database size. |

Global flags: `-C/--root <dir>` (project root) and `--json` (machine-readable output
everywhere). `search`, `trace` and `context` also take `-p/--project <id-or-name>`.

---

## Benchmarks

Measured by the harnesses in `e2e/`, on real fixtures from this repository — not quoted
from anywhere. Compression numbers are only meaningful together with **signal retention**,
so both are reported.

**Retrieval quality** — 25 labelled queries against the Python reference repo, BM25 leg
(`e2e/search_bench.py`):

| Top-1 | Top-3 | Top-5 | MRR |
|---|---|---|---|
| 15/25 (60%) | 18/25 (72%) | 19/25 (76%) | 0.882 |

Junk queries ("quantum banana teleport protocol") return **nothing**.

**Compression** (`e2e/compression_bench.py`, `e2e/compression_proxy_e2e.py`):

| Fixture | Tokens in → out | Saved | Signal kept |
|---|---|---|---|
| `json_array` (20 real search hits) | 31 565 → 9 815 | 68.9% | 20/20 items |
| `source_code` (`imc-search/lib.rs`) | 7 495 → 5 293 | 29.4% | 0 code lines lost |
| `build_log` (real cargo output) | 409 → 68 | 83.4% | every `ERROR` |
| `git_diff` (real commit) | 9 253 → 8 852 | 4.3% | 489/489 changed lines |

A real diff is mostly changed lines, so that number is low on purpose: anything much above
~5% would mean dropping the answer.

**Correctness** — the suite is the spec: 390 unit tests and 73 end-to-end checks, including
a live proxy test that inspects the payload actually sent upstream.

---

## Architecture

```
crates/
  imc-core        config, settings, gitignore-aware walk, SQLite store, prompt capture
  imc-parser      tree-sitter AST: symbols, call/import edges, AST-scoped chunks
  imc-index       walk → diff → parallel parse → chunk → embed → store → FTS
  imc-search      BM25 + graph + vector, RRF fusion, rerank
  imc-embed       embedding providers (OpenAI-compatible / opt-in hashing / none)
  imc-trace       call-hierarchy tracer (callees, callers, blast radius)
  imc-find        `imc read` and `imc context`
  imc-support     prompt library, endpoint bridge, context packing, search-limit policy
  imc-memory      project memory: CRUD, dedup, relevance ranking, extraction
  imc-proxy       transparent LLM proxy: routes, upstream providers, context injection,
                  tool-result compression and the CCR store
  imc-proxy-types wire types and provider config (Anthropic / OpenAI / Copilot)
  imc-proxy-copilot  Anthropic ⇄ Responses ⇄ Chat Completions translation
  imc-cli         the `imc` binary
web-dashboard/    Next.js + shadcn/ui dashboard
e2e/              test harnesses and benchmarks
```

**Storage.** One SQLite file per project: `files`, `symbols`, `edges`, `chunks`,
`embeddings`, `memories`, an FTS5 index over the chunks, plus the project registry. No
vector service, no graph service, no external database.

**Indexing.** Walk (gitignore + binary + size filters) → MD5 diff against `files` →
tree-sitter parse **on rayon threads** → single-threaded SQLite writes (WAL,
`synchronous=NORMAL`).

**Languages** (tree-sitter grammars): Python, JavaScript, TypeScript/TSX, Rust, Go, Java.
Markdown, JSON, YAML and TOML are indexed as text windows.

**Retrieval.** FTS5 with BM25 for exact identifiers, optional embeddings for paraphrase,
graph expansion for symbols, RRF to combine the legs, then a rerank that applies a
relevance floor — the same shape a good code-search engine uses, minus the server.

---

## Development

```bash
cargo test --release                                  # unit tests
python3 e2e/e2e_suite.py                              # end-to-end suite
python3 e2e/search_bench.py --repo /path/to/repo      # retrieval quality
python3 e2e/compression_bench.py                      # compression ratios + retention
python3 e2e/injection_e2e.py                          # live proxy injection check
cd web-dashboard && npm install && npm run dev        # dashboard on :3000
```

Clear the environment before running the harnesses so nothing reaches a real endpoint:

```bash
unset IMC_DATA_DIR IMC_CONFIG IMC_VECTOR IMC_EMBED_API_KEY OPENAI_API_KEY
```

Design notes, parity notes and JSON contracts live in [`docs/`](docs).

---

## License

MIT — see [LICENSE](LICENSE).
