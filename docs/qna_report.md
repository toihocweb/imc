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

There are two different things named "graph" in the retrieved context; the substantive answer is about the code-graph retrieval module.

**Graph loading/caching (`GraphSearcher._load_graph`)**
- It takes a graph file path, `stat()`s it, and builds a signature of `(st_mtime_ns, st_size)`. If a graph is already loaded and its signature matches, it returns the cached graph immediately ([`main/search/graph.py:46-75`]).
- Otherwise it acquires `self._graph_lock` and re-checks the signature (double-checked locking) before loading ([`main/search/graph.py:46-75`]).
- The file is read as JSON and parsed with `nx.node_link_graph(...)` into a networkx graph ([`main/search/graph.py:46-75`]).
- It builds…

**Citations**

- cited: `main/search/graph.py`, `tests/graph_indexing_test.py`, `ui/src/components/ProjectWorkspace.jsx`
- invalid: none

## `how search works`

**Retrieved**

1. `main/search/graph.py:200-201` — 0.0813
2. `main/search/semantic.py:92-133` — 0.0799
3. `main/search/kb.py:36-62` — 0.0792
4. `ui/src/components/SearchView.jsx:100-394` — 0.0572
5. `main/prompts/imc_search.md:1-63` — 0.0559

**Answer**

Search is exposed through three searcher classes, each with a `search(query, limit)` method that delegates to retrieval logic:

- `GraphSearcher.search` simply calls `self.retrieve(query, limit)` (`main/search/graph.py:200-201`).
- `Searcher.search` (semantic) checks whether the index exists, sets up embeddings, then builds a retriever: it starts with a vector retriever (`index.as_retriever(similarity_top_k=limit)`), and if a `bm25_store` directory exists, wraps it in a `QueryFusionRetriever` with `mode="reciprocal_rerank"` for hybrid Vector + BM25 + RRF search; otherwise it falls back to vector-only search. Failures are logged and return `[]` (`main/search/semantic.py:92-133`).
- `KBSearche…

**Citations**

- cited: `main/search/graph.py`, `main/search/semantic.py`, `main/search/kb.py`, `ui/src/components/SearchView.jsx`, `main/prompts/imc_search.md`
- invalid: none

## `how memory works`

**Retrieved**

1. `main/api/routers/memory.py:299-348` — 0.0574
2. `main/proxy/lib/imc_memory.py:667-810` — 0.0562
3. `main/api/routers/memory.py:284-295` — 0.0556
4. `ui/src/components/MemoryView.jsx:21-604` — 0.0543
5. `main/cli/commands/memory.py:21-132` — 0.0543

**Answer**

Memory in this codebase is a lifecycle-managed, project-scoped knowledge store. It supports create/read/update/delete, pruning of stale entries, feedback-driven lifecycle changes, and rank-based retrieval during queries. The CLI, HTTP API, and UI are the three interaction surfaces.

**Core storage and scoping**
- Memories live in a per-project database directory (`MemoryStore`), with `_global` as a special shared scope.
- The UI exposes three scopes: `project` (uses `activeProjectId`), `global` (uses `_global`), and `auto` (learned tool-failure patterns).
- `main/cli/commands/memory.py:32-33` and `main/cli/commands/memory.py:47-48`

**Create / Edit / Delete**
- The UI's add/edit modal posts …

**Citations**

- cited: `main/cli/commands/memory.py`, `ui/src/components/MemoryView.jsx`, `main/api/routers/memory.py`, `main/proxy/lib/imc_memory.py`
- invalid: none

## `how the proxy works`

**Retrieved**

1. `main/proxy/services/providers/provider_proxy.py:53-64` — 0.0461
2. `main/proxy/services/providers/provider_proxy.py:147-162` — 0.0447
3. `main/proxy/config.json:1-5` — 0.0402
4. `main/proxy/lib/api_config.py:126-135` — 0.0369
5. `main/proxy/lib/api_config.py:36-48` — 0.0355

**Answer**

The retrieved context shows only fragments of the proxy, so this is a partial picture.

**Provider proxying (request forwarding).** `forward_provider_alpha_search` builds upstream headers via `build_provider_upstream_headers`, reads the raw request body, resolves the upstream URL via `resolve_provider_request_url` (appending the query string and the path `/v1/alpha/search`), then POSTs to that URL with an `httpx.AsyncClient` and returns the result through `create_provider_proxy_response`.

**Response handling.** `create_provider_proxy_response` copies the upstream response headers into a dict, removes any header names listed in `STRIPPED_RESPONSE_HEADERS`, and returns a `Response` carrying t…

**Citations**

- cited: `main/proxy/services/providers/provider_proxy.py`, `main/proxy/config.json`, `main/proxy/lib/api_config.py`
- invalid: none

## `how indexing works`

**Retrieved**

1. `main/api/routers/indexing.py:24-42` — 0.0555
2. `main/api/routers/indexing.py:82-109` — 0.0531
3. `tests/graph_indexing_test.py:12-16` — 0.0480
4. `ui/src/components/ProjectWorkspace.jsx:34-165` — 0.0464
5. `main/indexing/semantic.py:73-293` — 0.0423

**Answer**

Indexing runs as a background task in two phases: a HTTP trigger that validates the target directory and marks the project as indexing, then an incremental semantic pipeline that hashes files, diffs them against a stored hash table, re-embeds only new/modified/deleted files, persists the vector store and docstore, and finally builds a BM25 index for hybrid search. Results stream to the UI over SSE.

**Citations**

- cited: `main/api/routers/indexing.py`, `main/indexing/semantic.py`, `tests/graph_indexing_test.py`, `ui/src/components/ProjectWorkspace.jsx`
- invalid: none

## `how trace works`

**Retrieved**

1. `main/cli/commands/trace.py:20-89` — 0.0584
2. `main/core/tracer.py:311-478` — 0.0569
3. `main/core/tracer.py:480-634` — 0.0541
4. `ui/src/components/ProjectWorkspace.jsx:34-165` — 0.0425
5. `tests/graph_schema_consumers_test.py:80-92` — 0.0297

**Answer**

Trace is implemented as a CLI command (`run_trace`) backed by `ASTTracer.trace_callees` (forward) and `ASTTracer.trace_callers` (reverse/blast radius). The flow is: validate project/graph and resolve target; choose direction (callers or `reverse` => reverse); if no candidates and not reverse, error; if JSON, emit per-mode trees; else print joined ASCII trees. For callers, the CLI preserves the selected definition identity by using `raw_target_name = candidates[0][0] if len(candidates)==1 else target` and passes `target_file=file_filter`.

Forward (`trace_callees`, node_id, depth, show_all, visited, current_depth): build tree from synthetic or graph node (name/file/line/type); return if `curr…

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

`Config` is a singleton configuration manager for Code Indexer that loads and caches configuration from a default YAML file. On first instantiation, `__new__` calls `_init_config`, which resolves the app directory, creates a centralized `data` directory inside the app, determines the target directory (from `active_project.txt` if present, otherwise the app dir, or an explicitly passed `target_dir` which it persists to that file), loads YAML from `DEFAULT_CONFIG_PATH` into `_config_data`, and sets database paths (`lancedb_dir`, `sqlite_db_path`) inside the app's data directory rather than the target directory. Configuration values are exposed as properties with getters (falling back to defaul…

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

The retrieved context shows several distinct, unrelated "context" mechanisms rather than one unified system, so a single end-to-end explanation is not supported. What the chunks do show:

- **CLI feature context:** `run_context(target, project, depth)` is the CLI handler for `imc context <target>`. It builds a dossier by calling `get_feature_context(target, project_id=project or "", depth=depth)` and prints it.
- **Request context:** `set_request_context(context)` stores a `RequestContext` in a context variable via `request_context_var.set(context)`.
- **Context packing (retrieval):** `pack_context` assembles chunks into a text block, deduplicating exact and suffix-overlapping content, honor…

**Citations**

- cited: `main/cli/commands/context.py`, `main/proxy/lib/request_context.py`, `tests/graph_retrieval_context_test.py`, `main/proxy/services/copilot/context_filters.py`, `ui/src/components/ProjectWorkspace.jsx`
- invalid: none

## `how claude integration works`

**Retrieved**

1. `main/cli/commands/claude.py:21-28` — 0.0613
2. `main/core/claude_sessions.py:26-50` — 0.0547
3. `ui/src/components/ClaudeSessionViewer.jsx:149-457` — 0.0499
4. `CLAUDE.md:1-95` — 0.0495
5. `ui/src/components/ClaudeCodeView.jsx:56-549` — 0.0494

**Answer**

## Overview

Claude integration in this codebase (IndexMyCode/IMC) operates on multiple levels: a CLI setup command, a proxy that intercepts Claude Code traffic, session tracking via `~/.claude/projects`, an API layer for session management, and a desktop terminal embedding Claude Code via Electron IPC.

### 1. CLI Project Setup

The `imc claude` command scaffolds Claude Code settings for the current project:

- `main/cli/commands/claude.py:L21-28` — `run_claude()` resolves the target directory and delegates to `_setup_claude_code(target_dir)` (from `main/cli/commands/index.py`), then prints a success message.
- `CLAUDE.md:1-95` documents this as: `imc claude` → "generate .claude/settings.lo…

**Citations**

- cited: `main/cli/commands/claude.py`, `main/cli/commands/index.py`, `main/proxy/main.py`, `main/core/claude_sessions.py`, `ui/src/components/ClaudeSessionViewer.jsx`, `ui/src/components/ClaudeCodeView.jsx`
- invalid: none

## `how compact works`

**Retrieved**

1. `main/cli/commands/compact.py:12-23` — 0.0613
2. `main/cli/commands/compact.py:162-167` — 0.0601
3. `ui/src/components/ProjectWorkspace.jsx:34-165` — 0.0563
4. `ui/src/components/ProjectsView.jsx:70-414` — 0.0259
5. `ui/src/components/ClaudeHistoryDrawer.jsx:54-526` — 0.0235

**Answer**

The compact feature works in two layers:

**CLI (`compact` command)** — dispatches on a subcommand:
- `save` → `_save(args.name)`
- `list` → `_list()`
- `use` → `_use(args.name)`
- `clear` → `_clear()`
- anything else prints "Unknown compact command."

The active compact is tracked via a file named `active_compact.txt` inside `db_dir`: `_get_active_compact` reads that file and returns its stripped contents, or `None` if the file doesn't exist.

**UI (Claude History Drawer)** — the "Compact" button on a session card calls `handleCompactSession`, which:
1. POSTs to `/claude/sessions/{sessionId}/prepare-compact` (with `project_id` / `project_path` params)
2. On success, builds the prompt ``Load…

**Citations**

- cited: `main/cli/commands/compact.py`, `ui/src/components/ClaudeHistoryDrawer.jsx`
- invalid: none

## Totals

- distinct paths cited: 36
- invalid citations: 0
- answers that opened by declaring partial coverage: 0/10
