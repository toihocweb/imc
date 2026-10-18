# Session handoff — 2026-10-18

Written when switching the agent model from DeepSeek to Qwen3.8-max. Everything
below was verified by running the tool, not inferred from reading code.

## Repos

- `/root/imc` — the **Python original**. Source of truth for behaviour.
- `/root/imc-rs` — the **Rust port**. 12 commits this session, HEAD `73bf658`.
- Binary: `imc-rs/target/release/imc` (plus `imc-proxyd`).

Rule: keep Rust behaviour identical to Python unless told otherwise.

## Verified state

```
unit tests   283 passed, 0 failed   (cargo test --release, whole workspace)
e2e suite     49 passed             (python3 e2e/e2e_suite.py)
build         0 errors              (cargo build --release)
```

The e2e suite covers python / typescript / rust / go / java on one fixture.

## Measured numbers (don't re-derive these)

`imc search` on `/root/imc` (25 labelled queries, `e2e/search_bench.py`):

- BM25 only — top-1 52%, top-3 76%, top-5 80%, MRR 0.785
- Hybrid — top-1 52%, top-5 84%, MRR 0.757
- Vector does **not** beat BM25 here. Say so rather than implying it does.

`main/prompts/imc_search.md` prompt tuning (`e2e/prompt_eval.py`, 3 runs, frozen
context — `docs/search_prompt_tuning.md` has the method):

| prompt | grounded (of 8) | key files | invalid citations |
|---|---|---|---|
| baseline (Python) | 5.3 | 7.3 | 4.3 |
| tuned (`docs/search_prompt_tuned.md`) | **8.0** | **8.3** | **0.0** |

`e2e/fe_be_link.py` — frontend→backend by string match, 35/36 endpoints across 6
React components, zero LLM calls.

## Uncommitted work (deliberate)

```
M crates/imc-cli/src/claude.rs        reminder to pass -C when cwd may not be the project
M crates/imc-proxy-types/src/types/anthropic.rs
?? crates/imc-proxy-types/tests/
```

The proxy change is **untagged -> tagged** on five serde enums. Untagged matched
the first variant, and every variant's fields were `#[serde(default)]`, so:

- every assistant message deserialized as a **user** message (role has a default)
- every `tool_use` block became an empty `Text`, losing id, name and input

Verified with a repro that round-trips a real payload. Tests in
`tests/anthropic_block_dispatch.rs`. Left uncommitted on request — say so rather
than assuming it should stay.

## Pending, not started

1. **`imc context` reports wrong paths for prefixed routers.** `git.py` has
   `APIRouter(prefix="/api/git")` and `@router.get("/status")`; the command prints
   `GET /status`, but the real route is `GET /api/git/status`. It re-implements
   route scanning instead of using `EndpointBridge`. Fix: use the bridge.
2. **`RouteBridge` is not wired to `imc trace`.** `EndpointBridge` is ported
   (`imc-support/src/endpoints.rs`, 1130 lines, has tests), the trait exists
   (`imc-trace/src/tree.rs`), `HttpRoute`/`ClientCall`/`KIND_ROUTE` exist, but no
   `impl RouteBridge for EndpointBridge` and `ImcTracer` holds only `NoBridge`.
   Roughly 60-80 lines of glue plus parsing `GET /api/x` targets at the CLI.

With #2 done, `fe_be_link.py` becomes redundant.

## Things that bit us, so they don't bite again

- `imc -C <repo>` — always pass it. Without it `resolve_root` prefers the globally
  active project over cwd, so an unrelated directory answers the query. This made
  a benchmark report 25/25 misses.
- `imc read --method` needs the **exact** name as printed by `--symbols`, including
  a leading underscore. `blended_score` fails, `_blended_score` works.
- `-l 50` returns 20 hits. That is Python parity (`min(max(n,1),20)`), not a bug.
- `imc trace <typo> -d callers` used to exit 0 with an empty tree, which an agent
  reads as "no callers". Fixed; now rc=1.
- Python's builtin list is applied to **Python callers only**. Applying it to Java
  silently hid `sum()` and `round()` from traces.
- `imc trace` resolves callees by bare name, so `User.describe` and
  `Checkout.describe` collide, and `Repository.findById` vs
  `UserRepository.findById` is ambiguous. Known limit, not fixed.
- The eval harness's citation checker must treat a path named *inside* a retrieved
  chunk as grounded. It previously flagged a correct citation as a hallucination.
- The prompt-eval labels were checked against the index: one asked for a language
  list the index does not contain, so the model refusing to cite it was correct.
