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
cargo test --release        # 373 tests, 0 failures
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
| `imc trace <target> [-d N] [--callers] [--callees] [-b/--both] [-a/--all] [-f file]` | Call tree / blast radius, also for `GET /api/path` route targets. Defaults to callees; `--callers` reverses, `--both` shows both directions. |
| `imc read <file> [--symbols] [--method NAME] [--offset N] [--limit N] [--raw]` | Token-optimised reader: blank-line strip, JSON smart-strip, auto-truncate with summary header. |
| `imc context <target>` | One-shot resolved context for a file/module (outline, deps, tests). |
| `imc claude setup\|wire\|start\|stop\|status` | Claude Code integration (see below). |
| `imc config get\|set\|list\|path\|test\|migrate\|setup` | Edit `~/.imc_config.yaml`: LLM url/key/model, embedding model. `setup` walks you through interactively. |
| `imc proxy serve\|status\|stop` | The LLM proxy Claude Code talks to. |
| `imc graph <symbol>` | Quick definitions / callees / callers dump. |
| `imc memory-add/-search/-list/-delete`, `imc learn <file>` | Project memory (rules, decisions). |
| `imc list` | Registered projects (files, symbols, edges, chunks, memories, DB size). |
| `imc pack "<query>" [--budget N] [--candidates N] [--no-vector] [--no-memories] [--graph N]` | Assemble an agent's context: retrieval + graph neighbourhood + project memories, compressed to a token budget; shortened items carry an `imc retrieve <id>` handle. |
| `imc compress [--text T] [--raw] [--store]` | Compress content by detected kind. Reads stdin when `--text` is absent. `--raw` prints the compressed text only; `--store` keeps the original and prints its id. |
| `imc retrieve <id> [--compressed]` | Print the original behind a compression handle (`--compressed` for the short form). |
| `imc ccr stats\|list\|clear\|prune` | Inspect the compression store: totals, recent records, drop everything, or expire old ones. |

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

## Context compression

The proxy sits between the agent and the model, so it sees every tool result
about to be re-read. Big ones are compressed on the way through and the
original is kept in a CCR store, so nothing is lost — the model gets a handle
and can pull the full text back verbatim.

Detection is automatic, by content not by filename:

| Kind | What it does |
|---|---|
| `json_array` | Collapses runs of identical items, elides a string field that duplicates a sibling (`text` vs `snippet`), windows fields over 1.5 KB to head+tail. Short arrays of distinct items are returned intact. |
| `build_log` | Keeps every `ERROR` / `WARN` / failure line and the diagnostic gutters; drops routine progress output. |
| `git_diff` | Dedupes identical file sections, elides the commit-message body, drops `index <hash>` lines, trims long unchanged runs. **Every changed line is preserved.** |
| `source_code` | String-literal-aware comment stripping (a `//` inside `"..."` is code), block-level repeat collapse, blank-line runs. |
| `plain_text` | Drops redundant sentences, keeps the first, the last, and the keyword-dense middle. |

Two invariants hold across all of them: **output is never larger than input**,
and an elision is always announced in place with a count.

### In the proxy

`/v1/messages` compresses tool results before forwarding. A result is only
touched when it is over 400 tokens **and** the compressor saves at least 20%;
`is_error` results are never touched, because the error text is the answer.

```
POST /v1/compress   {"content": "..."}          -> {id, kind, compressed, tokens, ratio}
POST /v1/retrieve   {"id": "..."}               -> {id, original, compressed}
GET  /v1/stats                                   -> totals + overall ratio
```

The store is `<data dir>/proxy_ccr.db` (`IMC_CCR_DB` overrides). The same store
is reachable from the terminal, so a handle in a log or a transcript is
actionable:

```bash
imc compress --text "$(cat build.log)" --store   # -> id 3139ac0529...
imc retrieve 3139ac052928fd49                     # exact original bytes
imc ccr stats                                     # totals + overall ratio
imc ccr list -l 10 --kind build_log
imc ccr prune --days 30                           # expire old handles
```

### Benchmarks

Both run against real data from this repo, and check **signal retention**
alongside the ratio — a compression number means nothing if the answer was in
what got dropped.

```bash
python3 e2e/compression_bench.py              # ratio + retention, real fixtures
python3 e2e/compression_proxy_e2e.py URL      # HTTP surface, byte-exact round-trip
python3 e2e/compression_request_path_e2e.py   # proxy compresses before forwarding
```

Measured on this repo:

```
json_array (20 real search hits)  31565 -> 9815 tokens   68.9%   20/20 items kept
source_code (imc-search/lib.rs)    7495 -> 5293 tokens   29.4%    0 code lines lost
build_log (real cargo output)       409 ->   68 tokens   83.4%    every ERROR kept
git_diff (real commit)             9253 -> 8852 tokens    4.3%    489/489 changed lines
```

A real diff is mostly changed lines. Anything much above ~5% there would mean
dropping the answer, so the number is low on purpose.

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
| `IMC_CCR_DB` | `<data dir>/proxy_ccr.db` | Compression store (originals for retrieval) |

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
  imc-proxy       transparent LLM proxy: routes, upstream providers, context injection,
                  tool-result compression + CCR store
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
