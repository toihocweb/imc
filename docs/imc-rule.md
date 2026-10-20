# imc — code-intelligence rules for this repo

You have access to the `imc` CLI (Rust port of IndexMyCode) in this repo.
Prefer `imc` over blind Read/Grep when you need to understand code, find
callers, or trace HTTP routes.

## Commands you should use

- `imc search "<3-7 words>"` — LLM answer from indexed chunks, with `path:line`
  citations. Needs `llm.base_url` configured. Without an LLM endpoint it prints
  ranked hits instead.
- `imc search "<q>" --raw` — raw ranked chunks, no LLM. Always read top 5,
  never stop at hit 1.
- `imc search "<exact symbol>"` — exact symbol lookup (case & `_`/camelCase
  normalized). Use when you know the name.
- `imc read <file> --symbols` — outline; then read only the lines you need.
- `imc context <file>` — whole feature around a file: deps, endpoints, tests.
- `imc trace <symbol> --callers --depth 3` — blast radius; run *before* you
  rename or delete anything.
- `imc trace "<symbol>"` — what it calls (forward). `--both` both directions.
- `imc trace "<GET /api/path>"` — HTTP route → backend handler.
- `imc graph <symbol>` — definition site and kind.
- `imc memory-search "<topic>"` — conventions/decisions recorded for this repo.

If MCP tools are available (`mcp__imc__search`, `mcp__imc__trace`, `mcp__imc__read`,
`mcp__imc__context`, `mcp__imc__memory_search`, `mcp__imc__pack`), prefer them over
shelling out: they carry the schema, the arguments are validated, and the project
root is pinned. Fall back to the CLI (`imc ...`) when MCP is not wired.
- `imc search "<q>" --on-miss` — if search returns nothing, the query is saved
  to memory so the next agentic pass can surface it.
- `imc trace "<symbol>" --callers` — if blast radius is large, a business_rule
  is auto-recorded to warn future editors.
- `imc ccr tune` — analyze compression store stats and suggest MIN_SAVINGS
  adjustments per content kind.
- `imc pack "<query>"` — assemble retrieval + graph + memory under a token
  budget. Shortened items carry an `imc retrieve <id>` handle.
- `imc list` — registered projects (files, symbols, edges, chunks, memories,
  DB size).

## Rules

1. **Search before you edit.** Never invent an API, path, or line number.
2. **Cite `path/to/file:L10-L25`** when answering. Never cite a range you did
   not see in an imc output.
3. **`imc trace` has no relevance floor.** A junk query still returns hits.
   Hits are ranked evidence, not proof — open the file before relying on it.
4. **If search misses repeatedly, fall back to Read/Grep.** An empty result
   means "not in the index", not "does not exist".
5. **Use `-C <project-root>`** (or `-p <id>`) when the cwd may not be the
   project; otherwise imc answers for the globally active project.

## Setup

- `imc config setup` — interactive setup of LLM URL, API key, models.
- `imc config test` — verify the embedding/chat endpoint.
- `imc claude setup` — index, wire `.claude/settings.local.json`, add this
  rule file, and start the proxy.
- `imc web` — open the web dashboard (starts it if needed); `--url` or
  `IMC_WEB_URL` for a remote host.
