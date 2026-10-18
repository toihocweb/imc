# IndexMyCode (Rust)

Full Rust port of the Python `imc` code-intelligence platform — no UI layer.
Same product idea, one binary, one SQLite file.

```
imc index .           # incremental index (AST + chunks + vectors)
imc search "query"    # hybrid BM25 + vector + call-graph search
imc graph symbol      # definitions / callees / callers
imc stats             # index statistics
imc projects          # registered projects
imc memory-add ...    # durable project memory
imc memory-search ... # recall memories
imc learn notes.md    # extract memories from a transcript
imc mcp               # MCP stdio server
```

Flags: `-C/--project <dir>`, `--json`, `index --force`, `index -i <pattern>`,
`search -l <n>`, `search --no-vector`.

## Workspace

| Crate | Replaces (Python) | What it does |
|---|---|---|
| `imc-core` | `main/ingest`, `main/core`, `main/project`, `main/indexing/*` storage | config, gitignore-aware walk, MD5 incremental hashes, SQLite store |
| `imc-parser` | `main/ingest/parser.py`, `main/indexing/graph.py` | tree-sitter symbols, call/import edges, AST-scoped chunks (Python, JS, TS, TSX, Rust, Go) |
| `imc-embed` | llama-index embeddings | OpenAI-compatible `/embeddings` + offline hashing fallback |
| `imc-search` | `main/search/*` | SQLite FTS5 BM25 + cosine vectors + RRF fusion + graph evidence |
| `imc-memory` | `main/memory/*` | memory CRUD, dedup, retrieval, rule-based extraction |
| `imc-cli` | `main/cli/*`, `main/mcp/*` | `imc` binary + MCP stdio server |

## Design changes vs the Python version

- **LanceDB + NetworkX + llama-index → one SQLite file** (tables: `files`,
  `symbols`, `edges`, `chunks`, `embeddings`, FTS5). No Python runtime, no
  vector-server process, incremental indexing still MD5-based.
- **BM25 comes from SQLite FTS5** (`bm25()`), vectors are f32 blobs scored with
  cosine in-process — fine to ~100k chunks; swap to `sqlite-vec` or HNSW beyond
  that.
- **Hybrid fusion uses Reciprocal Rank Fusion** (k=60) instead of llama-index's
  default ranker, so BM25 and vectors can disagree without a hand-tuned blend.
- **Embeddings degrade gracefully**: with `IMC_EMBED_API_KEY`/`OPENAI_API_KEY`
  set, embeddings hit an OpenAI-compatible endpoint; otherwise a deterministic
  local hashing embedder keeps indexing and retrieval fully offline.
- **Parallel parse, serial write**: tree-sitter extraction runs on rayon
  threads; SQLite writes stay single-threaded (WAL, NORMAL synchronous).

## Environment

| Variable | Default | Purpose |
|---|---|---|
| `IMC_DATA_DIR` | `~/.indexmycode-rs` | registry + per-project DBs |
| `IMC_EMBED_API_KEY` / `OPENAI_API_KEY` | unset | remote embeddings |
| `IMC_EMBED_BASE_URL` / `OPENAI_BASE_URL` | `https://api.openai.com/v1` | embeddings endpoint |
| `IMC_EMBED_MODEL` | `text-embedding-3-small` | embedding model |
| `IMC_MAX_FILE_BYTES` | `1048576` | skip files larger than this |

Per-project DB lands at `$IMC_DATA_DIR/projects/<md5(path)[:16]>/index.db`.

## Build & test

```
cargo build --release
cargo test --release
```

Binary: `target/release/imc` (~11 MB).

## Not ported (by design, this pass)

- `main/api/` (FastAPI + git/jira/copilot routers) — the Rust CLI reads the
  store directly; the HTTP surface is only needed by the Electron UI.
- `main/proxy/` (LLM proxy on `:12345`) — separate concern; the MCP server
  covers agent integration.
- `ui/`, `desktop/` — excluded per requirement (no UI).
- `main/core/prompts.py` + LLM-backed summarisation (`/api/ask`) — retrieval
  side is ported; LLM answering is intentionally left to the calling agent.
