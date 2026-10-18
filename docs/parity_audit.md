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
| context 14 | line numbers use Rust `lines()` (form-feed/NEL/LS are content), Python `splitlines()` splits on them | documented in the module; only affects files containing those separators |
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

search 3, 7, 9, 10, 11; trace 14, 15, 16, 18, 19; context 13, 17, 18. Rich vs
plain result rendering, `no matches` vs `No results found.`, stdout vs stderr for
the not-indexed error, Python's INFO/ERROR log lines interleaved on stdout,
`--json` node-type/id spellings, help/usage wording, and candidate ordering when
a name has several definitions.

## Open

- **Reverse trace of a route target.** Python has no better answer here: for
  `trace "GET /api/x" --callers` it prints the route node with no children. The
  port matches. The useful answer (which components call the route) is available
  the other way round: `trace <handler> --callers` walks
  handler ← route ← component.
- **`locate_target_file` semantic fallback** (context 4, above) — needs the
  embedder wired into `imc-find`.
- **Flask `@app.route("/x", methods=[...])`** — *resolved this session*: the
  bridge now reads it (one route per method, GET when `methods` is absent, with
  `Blueprint(url_prefix=...)` applied like a FastAPI `prefix=`). Neither
  implementation had it before, so a Flask service was invisible to `trace`,
  `context` and front-end linking; a unit test covers the decorator, the
  `methods` list, the tuple form and the default verb, and `/root/imc`'s 39
  FastAPI routes are still byte-identical to Python's list.
