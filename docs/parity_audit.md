# Parity audit: Rust port vs the Python original

Three parallel audits (one per command) compared the shipped binary against
`/root/imc` line by line and run by run, on the same repo, with `-C /root/imc`
on every call. 54 findings came back: 16 for `imc search`, 20 for `imc trace`,
18 for `imc context`. This file records what happened to each, so the analysis is
not redone. Dispositions:

- **FIXED** — the port was wrong; changed, with a test.
- **DELIBERATE** — the port differs on purpose; the reason is in the code and
  listed here. Every one of these is measured, not assumed.
- **SUPERSET** — the port accepts or reports something Python cannot. Nothing
  Python accepts is rejected.
- **COSMETIC** — wording, help text, or ordering only; no behaviour difference.
- **OPEN** — known, not fixed, with the reason.

## Fixed from the audit

| # | Finding | Fix |
|---|---|---|
| search 1 | `imc search` was retrieve-only while Python's default is agentic | `agentic.rs`, `-r/--raw`; e2e suite L |
| search 2 | `-l 0` accepted (fell back to 5); Python rejects at argparse | `positive_usize` parser, rc=2 |
| search 6 | short form was `-l`; Python's is `-n` | `-n` alias |
| search 8 | human output had no graph/AST evidence (Python printed an AST Context block) | `print_hits` prints the `// graph:` lines |
| trace 1 | `-d` meant depth in Python, direction here — a silent wrong answer | `-d/--depth`, direction is `--direction` |
| trace 2 | unknown `--direction` value ignored silently | clap value parser, rc=2 |
| trace 3 | `-r/--callers/--reverse/--up`, `-b/--both` missing | added |
| trace 4 | `-a` short for `--all` missing | added |
| trace 5 | `-p/--project <id_or_name>` missing | registry lookup |
| trace 6 | `callers` / `callees` / `blast` subcommands missing | added (`blast` is an alias) |
| trace 7 | `--depth 0` accepted | rejected, rc=2 |
| trace 11 | sibling call sites emitted one route node each | `emitted_routes` set, as Python's `visited` |
| trace 12 | file targets listed `type`/`enum`/`interface` sections | `FILE_TARGET_KINDS` |
| trace 13 | `impl Pair` shadowed `struct Pair` (same id, impl won the primary key) | parser walks impls without emitting a symbol |
| trace 17 | `--json` keys were sorted, Python keeps insertion order | `serde_json/preserve_order` |
| context 1 | `slice` alias, `-d/--depth`, `-p/--project` missing | added |
| context 2 | project resolution did not walk up from the cwd | registry walk in `resolve_root` |
| context 3 | `~` not expanded in targets | `expand_tilde` |
| context 5 | routes lost their `APIRouter(prefix=...)` | context uses `EndpointBridge` |
| context 6 | strict UTF-8 read failed on a stray byte; Python uses `errors="ignore"` | `read_text_lossy` |
| context 7 | related-test scan stopped differently mid-walk | stops after a directory, like Python |
| context 13 | Java sub-route regex excluded spaces, so `@GetMapping("/list all")` lost its sub-path (Python's `[^"')]*` accepts spaces) | fixed, unit test |
| context 8 | Rust-only `.cy.js` suffix | removed |
| context 10 | i18n JSON booleans showed `[missing]`; Python prints `True`/`False` | resolved |
| context 11 | nested i18n key could shadow its ancestor (sorted map) | `preserve_order` |
| context 16 | dossier had no trailing newline | `println!` |

Found outside the three audited commands, while checking the rest of the tool
against Python:

| Finding | Fix |
|---|---|
| `read --method <typo>` never suggested anything. The scorer only had substring matching, and an exact hit never reaches the not-found branch, so the list was always empty — Python also weighs camelCase/snake_token overlap and trigram Jaccard | ported the full scorer; e2e D6 |
| The suggestion hint said `imc-read`, Python's console script, which is not a command here | says `imc read` |
| `read --method` headers printed the absolute path where Python prints `path.relative_to(cwd)` | `display_path` |
| The not-found message printed the full path; Python prints `path.name` | `file_name` |

Not from the audit but found while fixing context 5: the bridge searched for the
handler from line 1 of the file instead of from the annotation
(`.take()` without `.skip(i)`), so every route in a file took the first `def` as
its handler, and `imc trace "GET /api/x"` jumped to the wrong function. Fixed in
all four scanners.

Also fixed while verifying trace: `const helper = () => …` was only indexed when
the name was uppercase, so every lowercase TS/JS helper and hook was invisible.
Python indexes them at top level whatever the case, and skips nested ones.

## Deliberate divergences (keep these)

| # | Divergence | Why |
|---|---|---|
| search 13 | reads `~/.imc_config.yaml`; Python reads only `~/.indexer_config.yaml` | the port reads the new name with a legacy fallback, and ships `imc config migrate` |
| search 14 | reranks fused hits with symbol/path/exact-identifier boosts | measured: BM25 top-1 44% → 52% after the boost work |
| search 15 | caps two chunks per file | without it one long file fills the whole answer |
| search 16 | FTS5 BM25 + cosine RRF instead of llama-index + BM25 | no Pydantic/llama-index stack in Rust; tuned separately (`docs/search_prompt_tuning.md`) |
| search 12 | always local; Python tries a running `imc serve` first | the port has no search server |
| trace 8 | typo in a target exits 1; Python prints an empty tree, rc=0 | agents read an empty tree as "no callers" |
| trace 9 | callers of an undefined name (builtin/stdlib) are reported; Python prints the bare name | measured on /root/imc and the e2e fixture — "nothing calls this" is a different claim from "this has no definition" |
| trace 10 | binds any name unique in the file *or* the project; Python binds same-file lexical/bare calls only | richer graph, needed for Java/Go/TS method calls that Python drops |
| context 4 | no semantic-index fallback in `locate_target_file` | needs an embedder + project handle; the filesystem scan and keyword scoring are ported |
| context 14 | line numbers used Rust `lines()`; Python's `splitlines()` also breaks on `\r`, `\v`, `\f`, `\x1c-\x1e`, NEL, U+2028/9 | fixed: `py_splitlines` |
| context 15 | `callers:` are bare short names from the store; Python prints `name [file:line]` | the store has no caller line for the ASTTracer path |

## Supersets (Python rejects, the port accepts)

- Global `--json` works for every command, including `search` and `trace`
  (Python only has per-subcommand `--json`; `search` has none). For `search`,
  `--json` implies `--raw`, so scripts never pay for an LLM call.
- `--no-vector` (BM25 only) — needed to run offline; Python always needs its
  embedding stack.
- `-C/--root <path>` for any command; Python resolves by project id/name only.
- `file::qualname` target syntax (Python takes `file:name`).
- i18n keys are also harvested from imported child components, and `.JSON` /
  `.PROPERTIES` (uppercase) translation files are picked up.

## Different shape, same capability

Python's `imc memory` is one command with flags (list by default, `-d <id>`
delete, `-c` clear, `-g` global, `-p` prune); this port ships them as
`memory-list`, `memory-delete`, `memory-prune`. Memory search and typed
`memory-add` are extra here — the Python CLI has no `memory search`. The names
are what the instruction block teaches, so agents see one spelling only.

## Cosmetic (no behaviour change)

search 3, 7, 9, 10, 11; trace 14, 15, 16, 18, 19; context 17, 18. Rich vs
plain result rendering, `no matches` vs `No results found.`, stdout vs stderr for
the not-indexed error, Python's INFO/ERROR log lines interleaved on stdout,
`--json` node-type/id spellings, help/usage wording, and candidate ordering when
a name has several definitions.

## A finding that did not survive checking

The trace audit reported, ten times over, that the port is missing Python's
"Skipping builtin call chain" diagnostic. It does not exist: `grep -rn "Skipping"
/root/imc/main --include=*.py` matches only unrelated KB/memory/parser log lines,
there is no `diagnostics.Skipping` call in `main/core/tracer.py`, and Python's
`imc trace` prints no such line. Nothing was implemented for it. Audit output is
a claim, not evidence — the same audit's other findings (the `-d` flag, route-node
duplication, impl/struct collision) all reproduced when run by hand, and those
were fixed.

## Open

- **Reverse trace of a route target.** Python has no better answer here: for
  `trace "GET /api/x" --callers` it prints the route node with no children. The
  port matches. The useful answer (which components call the route) is available
  the other way round: `trace <handler> --callers` walks
  handler ← route ← component.
- **`locate_target_file` semantic fallback** (context 4, above) — needs the
  embedder wired into `imc-find`.
- **The prompt-cache breakpoint sits on the last system block.** Anthropic's
  guidance puts the breakpoint at the end of the *stable* prefix, so a
  per-user system section stays cacheable across users; marking the last
  block moves the breakpoint past it and forfeits that sharing. Both
  implementations do it the same way (Python migrates the markers to the new
  last block as well), so this is parity, not a port bug — changing it means
  changing both. Left alone until someone measures the cache hit rate with
  and without it; the usage log now prints the rate, so that measurement is
  one command away.
- **Flask `@app.route("/x", methods=[...])`** — *resolved this session*: the
  bridge now reads it (one route per method, GET when `methods` is absent, with
  `Blueprint(url_prefix=...)` applied like a FastAPI `prefix=`). Neither
  implementation had it before, so a Flask service was invisible to `trace`,
  `context` and front-end linking; a unit test covers the decorator, the
  `methods` list, the tuple form and the default verb, and `/root/imc`'s 39
  FastAPI routes are still byte-identical to Python's list.

## Proxy + context injection parity (re-checked)

**HTTP surface — parity.** Python's proxy exposes exactly six routes
(`main/proxy/routes/`): `/health`, `/v1/quota`, `/v1/models`,
`/v1/hyperspace-models`, `/v1/chat/completions`, `/v1/messages`. The port
serves the same six (`crates/imc-proxy/src/routes/{system,models,chat,messages}.rs`)
plus three Rust-only compression routes (`/v1/compress`, `/v1/retrieve`,
`/v1/stats`) — class B, not a divergence. The Copilot flow is ported as its own
crate (`imc-proxy-copilot`), matching `main/proxy/services/copilot/`.

**Context injection — parity.** Python's `main/proxy/lib/imc_memory.py`
(1385 lines) ports to `crates/imc-proxy/src/imc_memory/lib.rs`. Every function
Python actually *calls* has a Rust counterpart:

| Python | Rust |
|---|---|
| `extract_workspace`, `get_system_text`, `get_conversation_user_query`, `get_last_user_text`, `get_all_session_text`, `extract_session_id` | same names |
| `inject_memories_into_system` (prepends `<codebase-context>`) | same, idempotent (refuses a second block) |
| `get_session_project` / `set_session_project` | same (session cache) |
| `detect_project` / `detect_subagent_info` | same, 7 detection sources |
| `apply_system_reminder_template` | same (prepend / override / no-op-when-disabled) |
| `attach_memory_to_message` (migrates `cache_control`) | same |
| `get_project_learned_memories` + `append_learned_memories_to_payload` | same, called from `routes/messages.rs:280` |
| `process_tool_outcomes`, `process_user_correction` | same, called from `routes/messages.rs:268-269` |
| `log_interaction_for_project` (writes `recent_interaction_logs.jsonl`) | same, called from `routes/messages.rs:884` |
| `get_auto_learning_context` | same |
| `is_main_request` | same — deliberately always `true` on both sides so injection does not depend on `cache_control` markers |

**Three Python functions have no Rust twin — and they are dead code in Python
too** (definition only, zero callers, checked with
`grep -rn "<fn>(" main/`): `recall_memories`, `_reinforce_memories_async`,
`bg_extract`. Not a regression; deliberately not ported.

**One behavioural difference (D).** Python caches a project's memories keyed by
the mtime of `memory_index/docstore.json` and drops the cache through
`invalidate_memory_caches()` when the memory API mutates a memory. The port
reads memories fresh from SQLite instead, so it can never serve stale
injections; the cost is a read per request. `invalidate_memory_caches` has no
Rust twin because the port ships no REST memory API.

**The real gap is the API server, not the proxy.** Python's `main/api/routers/`
exposes 115 REST routes (git 30, prompts 18, copilot 14, mcp 12, memory 12,
indexing 7, jira 7, kb 6, system 6, projects 3). The port ships no FastAPI
equivalent: agent access is provided by `imc mcp` (7 tools) and the web
dashboard instead. Class D, deliberate, but it is the one area where the two
implementations are not interchangeable.

Proxy test suite: 133/0, of which 40 cover memory/injection behaviour.

## Fixed: memory retrieval had no relevance floor (Rust-only bug, class B)

Found by running the real proxy against a stub upstream and inspecting the
payload it forwarded (`e2e/injection_e2e.py`).

**Symptom.** With one project memory stored, *every* request got that memory
injected — including queries with no term in common:

```
imc memory-search "what is the deployment pipeline for kubernetes"
  → 1 hit, score 0.761   (the stored memory was "Changelog entries use ISO 8601 dates")
imc memory-search "changelog date format"
  → 1 hit, score 0.750   ← the *matching* query scored LOWER
```

In the proxy log the unrelated turn still reported
`[IMC Memory] Selected 1 precise memories` and
`Injected 1 learned memories into role 'user'`.

**Cause.** Two defects in `crates/imc-memory`:

1. `lexical_scores()` expanded every query token into `"tok" OR "tok"*`
   without filtering. The two-letter word `is` prefix-matched `iso`, so one
   spurious hit made the max-normalised BM25 score 1.0.
2. `retrieve()` had no similarity floor. Python drops candidates below
   `SIMILARITY_CUTOFF = 0.55` (`main/memory/store.py`) before the blended
   re-rank; the port had none, so a memory with similarity 0 still scored
   ~0.75 on priority/recency/usage weights and passed the proxy's
   `has_relevance` gate (`semantic >= 0.50`).

**Fix.** `search_tokens()` (≥3 chars, stopword list) feeds the FTS leg, and
`retrieve()` applies Python's `SIMILARITY_CUTOFF = 0.55`.

**After.** Unrelated query → 0 hits and no injection; matching query → 1 hit
at 0.75. Regression test in `crates/imc-memory/tests/memory_retrieval.rs`
(`unrelated_query_does_not_return_a_memory`) and the live check in
`e2e/injection_e2e.py`.

## Fixed: one non-UTF-8 file aborted the whole index (class B)

`imc index` failed with `Error: stream did not contain valid UTF-8` and
indexed **zero** files when the tree contained a single file that was not
valid UTF-8. Python feeds tree-sitter raw bytes, so such a file still yields
symbols; only its text/embedding step degrades. `ingest::read_text` now
decodes lossily (`String::from_utf8_lossy`) instead of propagating the error.

## Chunking (class B — deliberate divergence, measured)

Python (`main/indexing/semantic.py`) chunks with LlamaIndex: `CodeSplitter`
for code files and `SentenceSplitter` for the rest. The Rust port chunks by
AST symbol instead.

Measured on the Python repo (183 files) with `e2e/search_bench_auto.py`
(120 questions generated from the repo's own docstrings, paired comparison):

| chunks | BM25 top-1 / top-5 / MRR | hybrid top-1 / top-5 / MRR |
|---|---|---|
| cAST split-then-merge (2000 non-whitespace chars) | 52 / 63 / 0.465 | 54 / 70 / 0.489 |
| one chunk per symbol | 58 / 77 / 0.543 | 62 / 79 / 0.563 |
| + contextual headers | **59 / 77 / 0.547** | **62 / 76 / 0.557** |

The cAST paper (arXiv 2506.15655) reports gains in *recall* and generation,
but on a per-symbol index the vector leg scores a single symbol instead of a
mixture of three, and that wins on precision. The strategy is therefore
configurable (`chunk_strategy`: `symbol` — default — or `cast`), and the
default is the one that measured better.

Two real bugs were fixed along the way:

* a symbol larger than the chunk budget was **truncated** at 4096 bytes, so
  the tail of the function was lost (it is now split into several chunks);
* the sliding window for files without AST symbols was **uncapped** and
  produced a single 8.4 KB chunk.

### Contextual headers

Splitting a symbol into parts has a cost: the middle of a function arrives as
a chunk that says `if prune:` and nothing else. Every part now carries the
symbol's signature and the first line of its docstring
(`run_memory (part 2/2) | def run_memory(...): | """Delete only
memories..."""`), so each hit is self-describing. This is the contextual
chunking trick — a few dozen characters of context that do not dilute the
query. The benchmark does not move (BM25 top-1 58 -> 59, hybrid 62 -> 62),
so the headers are kept for the agent, not for the score.

The text sent to the embedding model is capped at 4 KB on a character
boundary while the stored text stays whole, so the lexical leg still matches
the tail of a long function.

## Memory ranking: decay (class B — deliberate divergence, measured)

Python's memory ranking is `0.55*sim + 0.20*importance + 0.10*recency +
0.05*usage + 0.10*overlap`. The recency term has a weight of 0.10, which
cannot reorder a crowded result set: a memory used yesterday and one used
last month land within 0.1 of each other.

The port applies a decay as a separate multiplicative stage, with a 5x
spread: 1.5x used today, 1.2x this week, 0.8x this month, 0.5x this
quarter, 0.3x beyond. The combined score is multiplied before it is clamped,
so decay reorders while the public score stays in 0..1. The floor is 0.3:
decay is a bias, never a filter. Business rules and working methods are
exempt — a decision does not expire. Two tests in
`crates/imc-memory/tests/memory_retrieval.rs` cover both.

## Working-memory injection point (fixed — the port was wrong)

The port picked the **first** user turn; Python picks the **last** one
(`main/proxy/lib/imc_memory.py:953` scans `reversed(messages)`). So the
memory block was written into the oldest turn of the conversation while the
query sat at the other end — the worst zone for attention ("lost in the
middle", arXiv 2307.03172: 75.8% at the start, 53.8% in the middle) — and the
port also re-hydrated the block into the past turns, which Python does as
well.

Now both pick the most recent genuine user turn (a turn with real text, not
only tool results), and the re-hydration of the earlier turns is back: the
client resends the whole conversation every time, so a turn that carried a
block must carry the same bytes or the cached prefix changes. The
per-(session, project, query) cache keeps the block byte-identical between
turns. Covered by `crates/imc-proxy/src/imc_memory/tests.rs` and
`e2e/injection_e2e.py`.

## `imc pack` (superset)

Python has no `pack`. The port assembles retrieval, graph facts and
memories under a token budget. Two additions on top of the first version:
a relevance floor (a hit below a quarter of the best score is dropped) and
cross-source dedup (Jaccard >= 0.75 on the compressed text, because that is
what ships). Measured over 30 questions: the raw search results hold 58
near-duplicate pairs among 348 items, the pack holds none.

### The order of the bundle (measured, not assumed)

"Lost in the middle" says the ends of a long context are attended strongly,
which argues for putting the strongest item last. `e2e/pack_order_ab.py`
renders the same pack strongest-first and strongest-last, answers both with
the model, and checks the answer against the file the question came from
(36 questions, the run stopped there because the endpoint hung on the last
four).

| order | answer cites the right file |
|---|---|
| strongest first | 29/36 (80.6%) |
| strongest last | 29/36 (80.6%) |

Zero discordant pairs: no question was answered with one order and missed
with the other. The order does not move the needle for this workload, so the
order was left alone.
