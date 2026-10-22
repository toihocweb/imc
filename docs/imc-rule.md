# imc — code intelligence for this repo

This repo is indexed by `imc`: an AST symbol index, a call graph, a lexical/vector
index, and a durable memory store. Prefer `imc` over blind Read/Grep.

## Standard Workflow

1. **Orient**: Start with `imc pack "<task or feature>"` to get an all-in-one bundle of code chunks, call-graph connections, and repo conventions under a token budget.
2. **Find**: Use `imc search "<query>" --raw` for direct ranked code chunks (bypasses LLM summarization). Use `--concise` for a quick 1-line overview.
3. **Inspect**: Never read whole files. Run `imc read <file> --symbols` to see the outline, then `imc read <file> --method <name>` to read only the target function/method body.
4. **Impact check**: Before editing or deleting any function, run `imc trace <symbol> --callers --depth 3` to reveal the exact blast radius.
5. **Routes & Components**: Trace HTTP routes via `imc trace "GET /api/path"`, or inspect full component architecture via `imc context <file>`.

## What to run, when

| You want to | Run |
|---|---|
| get full task context in one shot | `imc pack "<query>"` |
| find code by concept or keyword | `imc search "<query>" --raw` |
| quick 1-line match preview | `imc search "<query>" --concise` |
| see file outline / list of methods | `imc read <file> --symbols` |
| read one function/method/class body | `imc read <file> --method <name>` |
| see what calls it before you change it | `imc trace <symbol> --callers --depth 3` |
| see what it calls (execution flow) | `imc trace <symbol>` |
| find an HTTP route's backend handler | `imc trace "GET /api/path"` |
| see whole feature (deps, hooks, tests) | `imc context <file>` |
| recall a decision this repo already made | `imc memory-search "<topic>"` |
| retrieve full uncompressed tool output | `imc retrieve <id>` |

## The commands

**`imc pack "<query>"`** — assemble the context an agent needs in one command:
hybrid retrieval plus call-graph neighborhood plus project memories, budgeted into tokens.
Use this instead of stacking manual search, read, and context calls.
- `--budget 12000` — token budget for the whole bundle (default 6000).
- `--rerank` — use LLM to score and rerank candidate chunks before packing.
- `--no-memories` — leave memories out if only code is needed.
- `--json` — machine-readable bundle.

**`imc search "<query>"`** — hybrid retrieval (BM25 + vector) over codebase chunks.
- `--raw` — **Recommended for agents**: returns raw ranked chunks directly without LLM summarization. Read the top 5, never stop at hit 1.
- `--rerank` — use LLM to score and rerank candidate chunks, outputting ranked code directly.
- `--concise` — one line per hit (~8x fewer tokens) to survey matches quickly.
- `-l 20` — how many chunks to retrieve (10 by default).
- `--offset N` — page through a long result set.
- `--no-vector` — BM25 only: no embedding call, offline and deterministic.
- `--json` — machine-readable hits.

**`imc read <file>`** — the token-optimised reader.
- `--symbols` — the outline. Start here.
- `--method <name>` — one symbol's body. **Use the real name from `--symbols`, including any leading underscore** (`_blended_score`, not `blended_score`).
- `--offset N -l 200` — a line range, for files without symbols or unindexed sections.
- `--json` — machine-readable.

**`imc trace <symbol>`** — the call tree, rooted at the definition line.
- `--callers --depth 3` — the blast radius. **Run this before you rename or delete anything.**
- `--both` — both directions (callees and callers).
- `imc trace "GET /api/path"` — an HTTP route to its backend handler.
- `-f <file>` — disambiguate when multiple files define the same symbol name.
- `--json` — machine-readable call tree.

**`imc context <file>`** — the whole feature around a file: component, deps,
endpoints, tests. `-d 2` resolves one more level of subcomponents.

**`imc memory-search "<topic>"`** — conventions and architectural decisions
already recorded for this repo.
- `--mem-type work_fact` — filter by memory type (`work_fact`, `work_task`, `work_method`, `business_rule`).
- `--scene <scene>` — filter by working scene.
- Empty is a valid answer: it means "no matching memory", not "this repo has no memories".

**`imc retrieve <id>`** — retrieve the original full text behind a compression handle.
When tool output is compressed with `[imc: ... full output is id <id>]`, run this to inspect the uncompressed original.

## Rules

1. **Search before you edit.** Never invent an API, path, or line number.
2. **Cite `path/to/file:L10-L25`** — and only a range you actually saw in an imc output.
3. **No blind whole-file reads.** Outline with `imc read <file> --symbols` first, then fetch functions with `imc read <file> --method <name>`.
4. **Check blast radius before edits.** Run `imc trace <symbol> --callers` before modifying shared functions.
5. **Ranked is not proven.** `imc trace` has no relevance floor: verify a hit before relying on it.
6. **Empty means "not in the index"**, not "does not exist". Fall back to Read/Grep.
7. **Use `-C <project-root>`** (or `-p <id>`) when the cwd may not be the project root.
8. Tool results are filtered before they reach you; large tool outputs carry retrieval handles (`imc retrieve <id>`).
