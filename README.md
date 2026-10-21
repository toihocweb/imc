# imc

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-2021?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Release](https://img.shields.io/github/v/release/toihocweb/imc?label=release)](https://github.com/toihocweb/imc/releases/latest)
[![Stars](https://img.shields.io/github/stars/toihocweb/imc?style=social)](https://github.com/toihocweb/imc/stargazers)
[![Downloads](https://img.shields.io/github/downloads/toihocweb/imc/total?label=downloads)](https://github.com/toihocweb/imc/releases)

**Code intelligence for coding agents.** `imc` indexes a repository with tree-sitter, keeps a
call graph and a lexical/vector index over it, and answers the questions an agent actually
asks — *"where is this handled?"*, *"what breaks if I change it?"*, *"what did we decide about
it?"*.

One Rust binary. No Python runtime. No vector database. One SQLite file per project.

```bash
imc search "authentication flow"      # answer with path:line citations
imc trace AuthController --callers    # blast radius before you rename
imc pack "how does login work"        # one token-budgeted context bundle
```

---

## Contents

- [Install](#install)
- [Quick start](#quick-start)
- [Capabilities](#capabilities)
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
cargo install --path crates/imc-cli --locked --no-strip   # → ~/.cargo/bin/imc
```

On macOS `--no-strip` matters: `cargo install` strips the binary, which invalidates its code
signature, and the kernel then kills it on launch (zsh prints `killed` with no message).
`imc install` and `imc update` re-sign what they write, so only a manual `cargo install`
needs the flag — or `codesign --force --sign - "$(which imc)"` afterwards.

Rust 1.80+ and a C compiler to build (tree-sitter grammars and the bundled SQLite are
compiled from source). The prebuilt installer needs neither.

---

## Quick start

```bash
imc config setup       # point it at a model endpoint — optional, raw hits work without it
imc .                  # index the current project (incremental; --force rebuilds)
imc search "how are sessions invalidated"
imc trace SessionStore.rotate --callers
```

For Claude Code, one command wires everything — index, `.claude/settings.local.json`, the
agent rule file, `.mcp.json`, and the proxy:

```bash
imc claude setup && claude
```

---

## Capabilities

### Search

Hybrid retrieval: BM25 (SQLite FTS5) + optional vector similarity, fused with RRF, plus
graph expansion around symbols.

```bash
imc search "tax calculation"             # LLM answer with citations
imc search "tax calculation" --raw       # ranked chunks only, no LLM
imc search "retry policy" --raw --concise  # one line per hit, ~8× fewer tokens
```

A junk query returns nothing rather than padding the list with plausible-looking noise. A
search that finds nothing is recorded as project memory so the next session can surface it.

### Call graph

```bash
imc trace AuthController.login --callers   # who calls it
imc trace "GET /api/login" --both          # HTTP route → handler, both directions
```

### Context packing

`imc pack` assembles retrieval, the graph neighbourhood and project memories into one bundle
that fits a token budget. Shortened items carry a handle you can expand later.

```bash
imc pack "why is login slow" --budget 6000
imc retrieve <handle>
```

### Memory

Durable, per-project knowledge: conventions, decisions, business rules.

```bash
imc memory-add "changelog dates are ISO 8601" --type business_rule
imc memory-search "changelog"
```

An empty `memory-search` result means *no matching memory*, not *this project has no
memories*.

### MCP server

Expose imc to any MCP client (Claude Code, Cursor, Cline, …) as tools with schemas — no shell
quoting, no guessing flags.

```bash
imc mcp install     # writes .mcp.json for this project
imc mcp             # serve over stdio
```

With the proxy running the same server answers at `POST http://127.0.0.1:12345/mcp`
(streamable HTTP). Tools: `search`, `ask`, `trace`, `read`, `context`, `memory_search`,
`pack`, plus resources `imc://project/{id}` and `imc://file/{path}`. Every call is pinned to
the project's root, so a tool call can never answer for a different repository.

### One command to start everything

```bash
imc            # starts the proxy (12345) and the dashboard (4789)
imc claude     # same, then hands over to the claude binary
```

`imc .` still indexes the current directory; `imc index <path>` indexes something else.
Indexing progress goes to stderr, so a long run is not a black box.

### Web dashboard

A shadcn/ui dashboard over the same index: projects, search, trace, memory, the
compression store, prompt captures, model mappings and the endpoint configuration.
The data is served by the proxy on port 12345 — the dashboard asks it over HTTP
instead of starting a process per click.

```bash
imc web     # opens http://localhost:4789, starting it if needed (--url for a remote host)
```

On a fresh clone the app has neither `node_modules` nor a build, so `imc web` installs and
builds it once (Node.js 18+). Light and dark themes, responsive down to phone width.

### LLM proxy and compression

The proxy sits between the agent and the model and compresses large tool results on the way
through; the original is kept in a CCR store, so the model gets a handle and can pull the
full text back verbatim.

| Kind | What it does |
|---|---|
| `json_array` | Collapses runs of identical items, elides a field that duplicates a sibling, windows fields over 1.5 KB to head+tail. |
| `build_log` | Keeps every `ERROR` / `WARN` / failure line and the diagnostic gutters; drops routine progress. A test runner's per-test line (`tests/test_api.py::test_login PASSED`) counts as a log line, so a whole pytest run is compressed instead of being mistaken for source code. |
| `source_code` | String-literal-aware comment stripping, repeat collapse, blank-line runs. |
| `git_diff` | Dedupes identical file sections, elides the commit-message body, trims long unchanged runs. **Every changed line is preserved.** |
| `plain_text` | Drops redundant sentences, keeps the first, the last and the keyword-dense middle. |

Two invariants hold across all of them: **the output is never larger than the input**, and
every elision is announced in place with a count. A result is only touched when it is over
400 tokens **and** the compressor saves at least 20%; `is_error` results are never touched.

```bash
imc compress --text "$(cat build.log)" --store   # -> id 3139ac0529…
imc retrieve 3139ac052928fd49                    # exact original bytes
imc ccr stats                                    # totals + overall ratio
```

The store is `<data dir>/proxy_ccr.db` (`IMC_CCR_DB` overrides). `POST /v1/compress`,
`POST /v1/retrieve` and `GET /v1/stats` expose the same thing over HTTP.

### Learned filters

Three commands ship with a rule (Cypress, Maven, GitHub Actions). For
everything else, imc writes the rule itself: when a command produces a large
result that no rule matched, the command and a sample of its output are kept
and the configured model is asked for the filter that keeps the signal and
drops the noise. The answer is a rule in the same format as the built-in
ones, so the next run of that command is filtered by it.

A learned rule is checked before it is stored: it has to shrink the output,
it must not empty it, its patterns have to compile, and it may not remove a
failure line. The model call runs in the background, off the request path.

A rule that was checked against one sample can still be wrong about the next
output of the same command, so the apply side has a net: after a learned rule
runs, a line that reads as a failure and is no longer there is put back, with
a note. A hand-written rule is never second-guessed.

```bash
cat ~/.indexmycode/learned_filters.json   # what has been learned
IMC_NO_LEARNED_FILTERS=1                   # turn the learning off
```

### Prompt capture

Every payload the proxy sends upstream is written verbatim — system prompt, tools, injected
memories, tool results. The fastest way to answer *"what did the agent actually send?"*

```bash
imc capture on && imc capture list && imc capture show system12
```

Off by default; toggleable from the dashboard at `/captures`.

### Staying current

`imc` checks for new releases in the background — once a day at most, never on the hot path —
and prints a one-line notice when one exists:

```
[imc] update available: v0.2.0 → v0.2.1 · run `imc update`
```

```bash
imc update            # download the newest release and replace the binary
imc update --check    # report the latest version and whether this platform has an asset
```

Disable the check with `IMC_NO_UPDATE_CHECK=1` (it is also off in CI).

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
imc config setup     # interactive
imc config list      # secrets masked
imc config test      # probe the endpoint
imc doctor           # diagnose everything, print the fix commands
```

**Which project imc answers for.** Without `-C`, imc walks up from the current directory to
the project that contains it. Outside any project it falls back to the globally active one
and says so. `imc index` never does that: without a path it indexes the current directory.

`imc doctor` reports the things that actually break setups — a missing `llm.model` (the
fallback is endpoint-specific and usually 404s), an unreachable endpoint, a project that was
never indexed, a proxy that is not running.

| Env | Default | Purpose |
|---|---|---|
| `IMC_CONFIG` | `~/.imc_config.yaml` | Config file path |
| `IMC_DATA_DIR` | `~/.indexmycode-rs` | Registry + per-project databases |
| `IMC_EMBED_API_KEY` / `OPENAI_API_KEY` | unset | Enables the vector leg |
| `IMC_EMBED_BASE_URL` | from `llm.base_url` | Embeddings endpoint |
| `IMC_EMBED_MODEL` | `text-embedding-3-small` | Embedding model |
| `IMC_MAX_FILE_BYTES` | `1048576` | Skip files larger than this |
| `IMC_CCR_DB` | `<data dir>/proxy_ccr.db` | Compression store |
| `IMC_NO_LEARNED_FILTERS` | unset | Set to any value to stop imc from learning filter rules |

Per-project database: `$IMC_DATA_DIR/projects/<md5(path)[:16]>/index.db`.

---

## Command reference

The commands that matter. Every command takes `--help`, and that is where the full flag
list lives.

| Command | What it does |
|---|---|
| `imc [path]` / `imc index [path]` | Index incrementally; `--force` rebuilds. `imc .` is the shortcut. |
| `imc search "<query>"` | Hybrid retrieval, then an LLM answer with `path:line` citations. |
| `imc trace <target>` | Call tree or blast radius (`--callers`), for symbols and `GET /api/…` routes. |
| `imc trace <symbol>` | The call tree from a symbol, rooted at its definition (kind, qualified name, line range). `--callers` for the blast radius, `--both` for both. |
| `imc read <file>` | Token-optimised reader; `--symbols` for the outline, `--method` for one function. |
| `imc context <target>` | Everything around a file: outline, dependencies, endpoints, tests. |
| `imc pack "<query>"` | Retrieval + graph + memories in one bundle under a token budget. |
| `imc memory-…` / `imc learn <file>` | Project memory: add, search, list, delete, prune; extract from a transcript. |
| `imc compress` / `retrieve <id>` / `ccr …` | Compress text, expand a handle, inspect the store. |
| `imc claude …` / `imc proxy …` | Claude Code integration; the LLM proxy on its own. |
| `imc mcp [install]` / `imc web` / `imc list` | MCP server, dashboard, registered projects. |
| `imc config …` / `imc doctor` / `imc update` / `imc install` | Configuration and model mappings, diagnostics, self-update, PATH install. |

Global flags: `-C/--root <dir>` (project root) and `--json` (machine-readable output
everywhere). `search`, `trace` and `context` also take `-p/--project <id-or-name>`.

---

## Benchmarks

Measured by the harnesses in `e2e/`, on real fixtures from this repository — not quoted from
anywhere. Compression numbers are only meaningful together with **signal retention**, so both
are reported.

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
  imc-memory      project memory: CRUD, dedup, relevance ranking, decay, extraction
  imc-proxy       transparent LLM proxy: routes, upstream providers, context injection,
                  tool-result compression and the CCR store
  imc-proxy-types wire types and provider config (Anthropic / OpenAI / Copilot)
  imc-proxy-copilot  Anthropic ⇄ Responses ⇄ Chat Completions translation
  imc-cli         the `imc` binary
web-dashboard/    Next.js + shadcn/ui dashboard
e2e/              test harnesses and benchmarks
```

**Storage.** One SQLite file per project: `files`, `symbols`, `edges`, `chunks`,
`embeddings`, `memories`, an FTS5 index over the chunks, plus the project registry. No vector
service, no graph service, no external database.

**Indexing.** Walk (gitignore + binary + size filters) → MD5 diff against `files` →
tree-sitter parse **on rayon threads** → single-threaded SQLite writes (WAL,
`synchronous=NORMAL`). Chunks are per AST symbol; a symbol larger than the budget is split
into parts that each carry the symbol's signature.

**Languages** (tree-sitter grammars): Python, JavaScript, TypeScript/TSX, Rust, Go, Java.
Markdown, JSON, YAML and TOML are indexed as text windows.

**Retrieval.** FTS5 with BM25 for exact identifiers, optional embeddings for paraphrase, graph
expansion for symbols, RRF to combine the legs, then a rerank that applies a relevance floor —
the same shape a good code-search engine uses, minus the server.

---

## Development

```bash
cargo test --release                                  # unit tests
python3 e2e/e2e_suite.py                              # end-to-end suite
python3 e2e/search_bench.py --repo /path/to/repo      # retrieval quality
python3 e2e/pack_order_ab.py                          # does bundle order matter?
unset IMC_DATA_DIR IMC_CONFIG IMC_VECTOR IMC_EMBED_API_KEY OPENAI_API_KEY
```

Clear that environment before running the harnesses, or they reach a real endpoint and a real
index. Design notes, parity notes and JSON contracts live in [`docs/`](docs).

**Keeping this repository clean.** `imc-rs` is a tool repository: it must never carry anything
from a real project — local paths, project names, captured prompts or API keys. A pre-commit
hook enforces it (`scripts/install_hooks.sh` once per clone; `scripts/repo_guard.py --all`
audits every tracked file). It refuses an absolute home path, a project name in
`scripts/repo_guard.deny` (shared) or `scripts/repo_guard.local.deny` (personal, gitignored),
a local artefact (`.env`, `*.db`, `captures/`, `.claude/settings.local.json`) or something
that looks like a real API key — and it checks the commit message too.

---

## Releasing

`imc update` installs the newest release asset for the running platform
(`imc-linux-x86_64`, `imc-linux-aarch64`, `imc-darwin-x86_64`, `imc-darwin-aarch64`). A push
to `master` publishes nothing; the assets are built when a `v*` **tag** is pushed.

Pushing a workflow file needs a token with the `workflow` scope. Without it, the macOS assets
are built on a Mac:

```bash
./scripts/release-macos.sh --dry-run   # build both architectures, sign, check
./scripts/release-macos.sh             # and upload them to the release
```

The script signs each binary ad-hoc (`codesign --force --sign -`): macOS kills an unsigned
Mach-O before it can print anything, which looks like a hung CLI. It also runs the binary it
just built, because a binary that cannot start is worse than no binary.

---

## License

MIT — see [LICENSE](LICENSE).
