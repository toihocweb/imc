# Session handoff — 2026-10-18/19

Continues from the previous handoff (which is now history; see `docs/parity_audit.md`
for the parity analysis this session produced). Everything below was verified by
running the tool, not read off the code.

## Repos

- `/root/imc` — the **Python original**, source of truth for behaviour.
- `/root/imc-rs` — the **Rust port**. Binary `target/release/imc`.
- Rule: keep Rust behaviour identical to Python unless a deviation is deliberate
  and recorded. `docs/parity_audit.md` holds that record, finding by finding.

## Verified state (end of session)

```
unit tests   315 passed, 0 failed   (cargo test --release, whole workspace)
e2e suite     64 passed             (python3 e2e/e2e_suite.py) — was 49
build         0 errors
```

The e2e run is now genuinely hermetic: it writes an empty `config.yaml` into its
temp data dir, so no embedding endpoint is touched. Before that fix it silently
fell back to `~/.indexer_config.yaml` (a missing `IMC_CONFIG` file is not an
empty config, it is a fallback to the legacy path).

e2e suites now: A index, B retrieval, C call graph, D reader, E memory, F claude,
G config, H proxy, I json, J multi-language, **K cross-boundary routes**,
**L agentic search** (stub LLM server, no network/key needed).

## Measured numbers

`imc search` on `/root/imc`, 25 labelled queries (`e2e/search_bench.py`):

| leg | top-1 | top-3 | top-5 | MRR |
|---|---|---|---|---|
| BM25 only | 52% | 76% | 80% | 0.785 |
| hybrid | 52% | 80% | **88%** | 0.745 |

BM25: top-1 14/25 (56%), top-3 18/25 (72%), top-5 19/25 (76%), MRR 0.838.
Hybrid: top-1 13/25 (52%), top-3 19/25 (76%), top-5 21/25 (84%), MRR 0.767
(after the chunk-cap fix; MRR improved 0.745 → 0.767, one labelled query moved
to CLI_COMMAND_AUDIT.md which is also a correct answer). Junk queries are
**empty in both legs** on both `/root/imc` and gakkaweo.

`e2e/qna_report.py` (10 questions, retrieval → prompt → LLM, mechanical citation
check): 5 chunks retrieved per question, **0 invalid citations, 0 partial
answers**, 32 distinct paths cited. The previous run cited 36 with two answers
empty/truncated (answers vary with the model; retrieval is unchanged).

Prompt tuning (`e2e/prompt_eval.py`, frozen context, 3 runs) re-measured after
the prompt's `-d callers` example became `--callers`: grounded 8.0/8, key files
9/13, 70 valid citations, 0 invalid, refusal handled 1/1. Unchanged.

## What this session changed (commit order)

```
bb50ea0 context: extract routes through EndpointBridge, and fix the handler scan it exposed
a17c660 trace: give the tracer a route bridge, so `GET /api/path` targets work
5c31156 search: answer through the LLM by default, and stop returning junk
2fe6f5a parser: index the declarations Python indexes, and not the ones it does not
22a8df9 cli: match Python's flags, and never silently ignore a wrong one
0aab33e parity: JSON document order, and the kinds a file target expands into
c1617b5 context: tolerate invalid UTF-8 like Python, and record the parity audit
1c3e83a HANDOFF: session state after the parity pass
97bb848 read: make --method suggestions work, and print paths like Python
10650bc cli/e2e: Python command aliases, -C path resolution, and a hermetic test run
93bb872 cli: name the offending path when -C does not resolve
da4d76c endpoints: read Flask's @app.route, which neither implementation saw
390edea context: restore the Java sub-path Python keeps, and match splitlines
164dbeb docs: qna_report answers are model output, not a fixture — say which numbers are stable
<NEW> real-repo verification on RabeMaster/gakkaweo (React Vite + Java Spring Boot + Python ai-service)
<NEW> parser/search byte-slice panics on non-ASCII, fixed + regression tests
<NEW> gitignore honoring via `ignore` crate; `.egg-info` pollution out of the index
<NEW> graph not-found rc=1; settings mask / models `strip_trailing_date` non-ASCII fixes
<NEW> docs/real_repo_verification.md — the full sweep report
```

The two items the last handoff listed as pending are both done: prefixed-router
paths in `imc context`, and `RouteBridge` wired to `imc trace`. `e2e/fe_be_link.py`
is now redundant (the tracer does that linkage itself) but is left in place as a
Python-side cross-check.

Biggest additions: agentic `imc search` (the port's first LLM path, with a
hermetic e2e suite), the full Python trace/context CLI surface, the parser fixes
that were hiding structs (id collision with `impl`) and every lowercase TS/JS
arrow helper, and a working `read --method` suggestion scorer (it was always
empty). `docs/parity_audit.md` has the per-finding record.

Outside the three audited commands, `read` was compared against Python too:
suggestion scoring, `-C` path resolution, relative-path headers, `path.name` in
the not-found message, and the `find`/`list`/`status` aliases all came from that
pass. The last change is Flask: `@app.route(...)` is now a route — nothing read
it before, in either implementation, so a Flask service was invisible.

## Uncommitted, deliberately

```
M crates/imc-proxy-types/src/types/anthropic.rs   untagged -> tagged on five serde enums
?? crates/imc-proxy-types/tests/                  the repro that proves it
```

Left uncommitted on request last session and still uncommitted now; its tests
pass. The change fixes real corruption (every assistant message deserialised as a
user message; every `tool_use` block became an empty `Text`, losing id, name and
input). Say so rather than assuming it should stay or go.

## Verified cross-boundary behaviour (new)

On `/root/imc`, byte-identical to Python:

```
imc trace "GET /api/git/status"      -> route node -> handler get_git_status -> its calls
```

On a fixture with a React caller:

```
trace <component>          component -> HTTP route -> backend handler
trace <handler> --callers  handler <- HTTP route <- component
trace "GET /api/nope"      rc=1, "Symbol ... not found"
```

## Things that bit us, so they don't bite again

- `imc -C <repo>` — always pass it. Without it the port answers for the active
  project or a registry match from the cwd (both new behaviours), not necessarily
  the directory you meant. `e2e/search_bench.py` pins `-C` for this reason.
- Shell exports persist across `bash` tool calls. A stray `export IMC_DATA_DIR`
  made an earlier probe read another project's DB and report "no such table:
  chunks_fts". `unset IMC_DATA_DIR IMC_CONFIG IMC_VECTOR` before comparing
  against `~/.imc_config.yaml`.
- `chunks_fts` is created by `imc index`, not by `store::open`. A unit test that
  builds a store by hand must call `imc_search::ensure_fts` first.
- Python's `search` on this machine finds nothing at all: it reads
  `~/.indexer_config.yaml`, which does not exist here, so its vector table is
  uninitialised. "Python says No results found" is therefore not evidence about
  relevance — check `imc config path` before drawing a parity conclusion from it.
- The LLM endpoint is configured with `base_url https://api.xah.io/v1` and
  **no** `llm.model`, so the default `gemini-3.1-flash-lite` 404s ("model is not
  available"). `claude-haiku-4.5` works. `imc search` now prints the response body
  and says which model it tried, so this is a five-second diagnosis rather than a
  mystery.
- `imc context -d/--depth` is accepted and ignored: Python's
  `get_feature_context(..., depth)` never reads it either (main/cli/commands/
  context.py:555-681).
- A subcommand-level `--project` and a global one share clap's arg id and clap
  **panics** on the type mismatch. The global path flag is `-C/--root` for that
  reason; do not rename it back.
- `imc trace <typo> --callers` exits 1 (parity break kept deliberately); the
  Python original prints an empty tree and exits 0.
- When comparing Rust and Python output, run both on the *same* repo and check
  who is right before "fixing" the port: several audit findings turned out to be
  Python bugs the port had faithfully copied (the route prefix), and one turned
  out to be the port being deliberately better (undefined-name callers).
- `~/.indexer_config.yaml` exists on this machine as well as `~/.imc_config.yaml`
  (both hold the same key). A missing config file does **not** mean "no config":
  the loader falls back to the legacy path, so anything that wants a clean
  environment must create an empty file at `IMC_CONFIG`.
- Python's own search is unusable here (it reads `~/.indexer_config.yaml` for its
  *Pydantic* config but its vector store was never initialised), so Python cannot
  be the oracle for search behaviour on this machine; use the labelled benchmark
  and the junk-query set instead.
- `imc find` / `imc list` / `imc status` now resolve to `read` / `projects` /
  `stats`, so a doc that says `imc find` is correct again.
