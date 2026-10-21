# imc — code intelligence for this repo

This repo is indexed by `imc`: an AST index, a call graph, a lexical/vector
index and a project memory store. Prefer `imc` over blind Read/Grep.

## What to run, when

| You want to | Run |
|---|---|
| know what some code does | `imc search "<3-7 words>"` |
| find where something is defined | `imc search "<exact symbol>"`, or `imc read <file> --symbols` |
| see one function's body | `imc read <file> --method <name>` |
| know what calls it, before you change it | `imc trace <symbol> --callers --depth 3` |
| know what it calls | `imc trace <symbol>` |
| find an HTTP route's handler | `imc trace "GET /api/path"` |
| get a lot of context at once | `imc pack "<query>"` |
| recall a decision this repo already made | `imc memory-search "<topic>"` |
| see the whole feature around a file | `imc context <file>` |

## The commands

**`imc search "<query>"`** — hybrid retrieval (BM25 + vector) over the chunks.
It answers with an LLM summary citing `path:line` (needs `llm.base_url`
configured); without an LLM it prints the ranked hits instead.

- `--raw` — the ranked chunks, no LLM. **Read the top 5, never stop at hit 1.**
- `--concise` — one line per hit (~8x fewer tokens); then read the file.
- `--json` — machine-readable hits.
- `--no-vector` — BM25 only: no embedding call, offline and deterministic.
- `--offset N` — page through a long result set; the output says when more
  matches exist.
- `-l 20` — how many chunks to retrieve (10 by default).
- An exact identifier is matched exactly (case and `_`/camelCase normalised).

**`imc read <file>`** — the token-optimised reader.

- `--symbols` — the outline. Start here.
- `--method <name>` — one symbol's body. **Use the real name from `--symbols`,
  including any leading underscore** (`_blended_score`, not `blended_score`).
- `--offset N -l 200` — a line range, for the part the outline did not answer.
- `--json` — machine-readable.

**`imc trace <symbol>`** — the call tree, rooted at the definition: its first
line carries the kind, the qualified name and the line range.

- `--callers --depth 3` — the blast radius. **Run this before you rename or
  delete anything.**
- `--both` — both directions. `--json` — machine-readable.
- `imc trace "GET /api/path"` — an HTTP route to its backend handler.

**`imc pack "<query>"`** — one bundle under a token budget: retrieval plus the
call-graph neighbourhood plus the project memories. Use it instead of stacking
search + read + context. `--budget 12000` for more, `--no-memories` to leave
the memories out.

**`imc memory-search "<topic>"`** — the conventions and decisions already
recorded for this repo. `--mem-type work_fact`, `--scene <scene>` to narrow it.
Empty is a valid answer: it means "no matching memory", not "this repo has no
memories".

**`imc context <file>`** — the whole feature around a file: component, deps,
endpoints, tests. `-d 2` resolves one more level of subcomponents.

## Rules

1. **Search before you edit.** Never invent an API, path or line number.
2. **Cite `path/to/file:L10-L25`** — and only a range you actually saw in an
   imc output.
3. **Ranked is not proven.** `imc trace` has no relevance floor at all: a junk
   query still returns hits. Open the file before you rely on a hit.
4. **Empty means "not in the index"**, not "does not exist". Fall back to
   Read/Grep.
5. **Use `-C <project-root>`** (or `-p <id>`) when the cwd may not be the
   project — otherwise imc answers for the globally active project and says so
   on stderr. `imc index` without a path always indexes the current directory.
6. Tool results are filtered before they reach you, and imc learns a filter
   per command: a long build log arrives as its errors and its summary, not as
   400 progress lines.
