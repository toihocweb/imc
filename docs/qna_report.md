# `imc search` + LLM — end-to-end answers for 10 questions

Model `claude-haiku-4.5`, retrieval top-5 from the real index of `/root/imc` (183 files), prompt = `docs/search_prompt_tuned.md`.

Each entry shows what retrieval actually returned, then the model's answer,
then a mechanical check of every cited path against the context that was sent.

## `how graph works`

**Retrieved**

1. `ui/src/components/ProjectWorkspace.jsx:34-165` — 0.0582
2. `main/search/graph.py:46-75` — 0.0577
3. `tests/graph_indexing_test.py:24-26` — 0.0564
4. `tests/graph_indexing_test.py:19-21` — 0.0557
5. `main/search/graph.py:81-198` — 0.0552

**Answer**

The graph works as a two-stage retrieval pipeline: a cached graph load, then semantic search with AST annotation.

**Loading (`_load_graph`)**: Loads `graph_store.json` via `nx.node_link_graph`, using an mtime/size signature for cache invalidation (double-checked under a lock). It builds three lookup indexes in one scan: `_file_entities` (file → definition nodes), `_name_entities` ((path, name) → nodes), and `_range_entities` ((path, "line"/"start_char") → sorted entries) for range queries.

**Retrieval (`retrieve`)**: Requires `graph_store.json` to exist (else logs an error and returns `[]`). It runs `self.searcher.search(query, limit)` for base semantic results (capped at 25), then for eac…

**Citations**

- cited: `main/search/graph.py`, `tests/graph_indexing_test.py`
- invalid: none

## `how search works`

**Retrieved**

1. `main/search/graph.py:200-201` — 0.0813
2. `main/search/semantic.py:92-133` — 0.0799
3. `main/search/kb.py:36-62` — 0.0792
4. `ui/src/components/SearchView.jsx:100-394` — 0.0572
5. `main/prompts/imc_search.md:1-63` — 0.0559

**Answer**

Search is implemented as multiple retriever backends plus a chat UI that drives them.

- **Semantic codebase search** (`main/search/semantic.py:92-133`): `Searcher.search` returns `[]` if the index dir `self.db_dir` does not exist, otherwise it calls `self.setup_embedding()`, gets a vector index via `self._get_vector_index()`, and builds `index.as_retriever(similarity_top_k=limit)`. If `self.db_dir / "bm25_store"` exists, it tries to build a BM25 retriever and swaps in a `QueryFusionRetriever` with `[vector_retriever, bm25_retriever]`, `num_queries=1`, `mode="reciprocal_rerank"`; it logs `BM25 Index found. Using Hybrid Search (Vector + BM25 + RRF).` If BM25 is absent it logs `BM25 Index not …

**Citations**

- cited: `main/search/semantic.py`, `main/search/graph.py`, `main/search/kb.py`, `ui/src/components/SearchView.jsx`
- invalid: none

## `how memory works`

**Retrieved**

1. `main/api/routers/memory.py:299-348` — 0.0574
2. `main/proxy/lib/imc_memory.py:667-810` — 0.0562
3. `main/api/routers/memory.py:284-295` — 0.0556
4. `ui/src/components/MemoryView.jsx:21-604` — 0.0543
5. `main/cli/commands/memory.py:21-132` — 0.0543

**Answer**

Memory in this codebase is a project-scoped (and global) store with retrieval, lifecycle events, and UI/CLI surfaces. It stores text memories with metadata, ranks them for queries, records usage/events, and supports lifecycle transitions and invalidation.

#### Storage & scope
- Memories are stored per project (and `_global`) via `MemoryStore` in the project DB directory. The CLI resolves scope: if `--global`, uses `_global` DB dir; else finds the indexed project and uses its DB dir (`main/cli/commands/memory.py:21-132`).
- API endpoints resolve store/project via `_resolve_store_and_project` and operate on that store (`main/api/routers/memory.py:191`, `main/api/routers/memory.py:284-295`, `m…

**Citations**

- cited: `main/cli/commands/memory.py`, `main/api/routers/memory.py`, `ui/src/components/MemoryView.jsx`, `main/proxy/lib/imc_memory.py`
- invalid: none

## `how the proxy works`

**Retrieved**

1. `main/proxy/services/providers/provider_proxy.py:53-64` — 0.0461
2. `main/proxy/services/providers/provider_proxy.py:147-162` — 0.0447
3. `main/proxy/config.json:1-5` — 0.0402
4. `main/proxy/lib/api_config.py:126-135` — 0.0369
5. `main/proxy/lib/api_config.py:36-48` — 0.0355

**Answer**

Partial answer: the retrieved context shows only fragments of the proxy — provider request forwarding and header preparation — not an end-to-end explanation of how the proxy works.

What the code does, based on the context:
- For provider endpoints (e.g. `/v1/alpha/search`), the proxy builds upstream headers from the provider config and the incoming request, resolves a target URL, POSTs the body via `httpx.AsyncClient`, and returns the upstream response with certain headers stripped (`STRIPPED_RESPONSE_HEADERS`).
- For message-proxy traffic, `prepare_message_proxy_headers` injects request IDs (`x-agent-task-id`, `x-request-id`), intent/type headers, and a user agent — unless running as the o…

**Citations**

- cited: `main/proxy/services/providers/provider_proxy.py`, `main/proxy/lib/api_config.py`, `main/proxy/config.json`
- invalid: none

## `how indexing works`

**Retrieved**

1. `main/api/routers/indexing.py:24-42` — 0.0555
2. `main/api/routers/indexing.py:82-109` — 0.0531
3. `tests/graph_indexing_test.py:12-16` — 0.0480
4. `ui/src/components/ProjectWorkspace.jsx:34-165` — 0.0464
5. `main/indexing/semantic.py:73-293` — 0.0423

**Answer**

Indexing runs as an API-triggered background pipeline:

- Trigger: `trigger_index` resolves and validates the target directory, adds the project, guards against a concurrent run via the `is_indexing` status, sets `is_indexing: True`, and schedules `server_module.run_indexing` with `background_tasks.add_task`. `main/api/routers/indexing.py:24-42`
- Core pipeline (`Indexer::build_index`): initializes the DB and storage paths, loads existing hashes from `file_hashes`, reuses a supplied `manifest` or rescans with `FileIngester`, classifies files as new/modified/unchanged plus deleted, and returns early if there are no changes. If changes exist, it sets up embeddings, loads or creates the LanceDB…

**Citations**

- cited: `main/api/routers/indexing.py`, `main/indexing/semantic.py`, `ui/src/components/ProjectWorkspace.jsx`, `tests/graph_indexing_test.py`
- invalid: none

## `how trace works`

**Retrieved**

1. `main/cli/commands/trace.py:20-89` — 0.0584
2. `main/core/tracer.py:311-478` — 0.0569
3. `main/core/tracer.py:480-634` — 0.0541
4. `ui/src/components/ProjectWorkspace.jsx:34-165` — 0.0425
5. `tests/graph_schema_consumers_test.py:80-92` — 0.0297

**Answer**

`imc trace` is implemented in two layers: a CLI entry point and an `ASTTracer` core.

**CLI layer (`run_trace`)**
- Creates an `ASTTracer(project_id=project)` and exits with an error if no indexed project or no AST graph exists.
- Resolves the target via `tracer.resolve_target(target, file_filter)`.
- Determines direction: `is_reverse = callers or reverse`. If no candidates are found and it is not a reverse trace, it errors out.
- In JSON mode it builds a result object with `project`, `target`, `depth`, and `results`; it appends a `"callees"` tree when `both or not is_reverse`, and a `"callers"` tree when `both or is_reverse`, then prints `json.dumps(..., indent=2)`.
- In text mode it does t…

**Citations**

- cited: `main/cli/commands/trace.py`, `main/core/tracer.py`, `tests/graph_schema_consumers_test.py`
- invalid: none

## `how config works`

**Retrieved**

1. `main/core/config.py:12-189` — 0.0819
2. `main/proxy/lib/types/chat_completions.py:191-192` — 0.0624
3. `ui/src/components/ProjectWorkspace.jsx:34-165` — 0.0547
4. `mcp_config_example.json:1-9` — 0.0519
5. `ui/vite.config.js:1-11` — 0.0518

**Answer**

`Config` is a singleton configuration manager for Code Indexer that loads and caches configuration from a default YAML file. On first instantiation, `__new__` calls `_init_config`, which resolves the app directory, creates a central `data` directory, determines the target (indexed) directory — from `active_project.txt` if present, an explicit `target_dir` argument, or the app dir as fallback — and writes the active project file when a target is given. It then loads YAML from `DEFAULT_CONFIG_PATH` into `_config_data` (errors are logged, not raised), and always stores databases (`lancedb_data`, `file_hashes.db`) inside the app's data directory rather than the target directory. Settings are exp…

**Citations**

- cited: `main/core/config.py`
- invalid: none

## `how context works`

**Retrieved**

1. `ui/src/components/ProjectWorkspace.jsx:34-165` — 0.0576
2. `main/cli/commands/context.py:679-682` — 0.0565
3. `main/proxy/lib/request_context.py:23-24` — 0.0562
4. `tests/graph_retrieval_context_test.py:116-128` — 0.0548
5. `main/proxy/services/copilot/context_filters.py:196-210` — 0.0533

**Answer**

The retrieved context shows four distinct "context" mechanisms:

1. **CLI context command** — `imc context <target>` calls `run_context`, which delegates to `get_feature_context(target, project_id=..., depth=depth)` and prints the resulting dossier `main/cli/commands/context.py:679-682`.
2. **Request-scoped context (proxy)** — `set_request_context` stores a `RequestContext` object into a `ContextVar` (`request_context_var.set(context)`), i.e. per-request state propagation `main/proxy/lib/request_context.py:23-24`.
3. **Context packing for retrieval** — `pack_context` merges duplicate/overlapping chunks: a repeated line appears once, exact duplicates collapse, a token budget of 0 yields empty…

**Citations**

- cited: `main/cli/commands/context.py`, `main/proxy/lib/request_context.py`, `tests/graph_retrieval_context_test.py`, `main/proxy/services/copilot/context_filters.py`
- invalid: none

## `how claude integration works`

**Retrieved**

1. `main/cli/commands/claude.py:21-28` — 0.0613
2. `main/core/claude_sessions.py:26-50` — 0.0547
3. `ui/src/components/ClaudeSessionViewer.jsx:149-457` — 0.0499
4. `CLAUDE.md:1-95` — 0.0495
5. `ui/src/components/ClaudeCodeView.jsx:56-549` — 0.0494

**Answer**

Claude Code integration works at three layers:

1. **CLI setup**: `imc claude` calls `_setup_claude_code(target_dir)` for the resolved current directory to generate the Claude Code configuration, then reports "Claude Code integration configured." Per CLAUDE.md, this creates `.claude/settings.local.json` for a project.

2. **Session tracking**: The backend resolves the Claude Code session store by mapping a workspace path to `~/.claude/projects/<slug>` — the path is normalized and slugified (`re.sub(r'[^a-zA-Z0-9]', '-', norm)`), with a fallback that matches directories by base filename if the exact slug doesn't exist. This directory is used by session list/detail/export/learn endpoints.

3. …

**Citations**

- cited: `main/proxy/main.py`, `main/cli/commands/claude.py`, `main/core/claude_sessions.py`, `ui/src/components/ClaudeCodeView.jsx`, `ui/src/components/ClaudeSessionViewer.jsx`
- invalid: none

## `how compact works`

**Retrieved**

1. `main/cli/commands/compact.py:12-23` — 0.0613
2. `main/cli/commands/compact.py:162-167` — 0.0601
3. `ui/src/components/ProjectWorkspace.jsx:34-165` — 0.0563
4. `ui/src/components/ProjectsView.jsx:70-414` — 0.0259
5. `ui/src/components/ClaudeHistoryDrawer.jsx:54-526` — 0.0235

**Answer**

The context shows two separate "compact" mechanisms, and only partially covers how each works:

1. **CLI `compact` command** (`main/cli/commands/compact.py`): `run_compact` dispatches on a subcommand — `save` (calls `_save(args.name)`), `list` (calls `_list()`), `use` (calls `_use(args.name)`), `clear` (calls `_clear()`), else prints "Unknown compact command." The active compact is resolved by `_get_active_compact(db_dir)`, which reads `db_dir / "active_compact.txt"` and returns its stripped contents, or `None` if the file doesn't exist. The bodies of `_save`, `_list`, `_use`, and `_clear` are not in the retrieved context, so what "save"/"use"/"clear" actually do to data is not covered.

2. …

**Citations**

- cited: `main/cli/commands/compact.py`, `ui/src/components/ClaudeHistoryDrawer.jsx`
- invalid: none

## Totals

- distinct paths cited: 32
- invalid citations: 0
- answers that opened by declaring partial coverage: 0/10
