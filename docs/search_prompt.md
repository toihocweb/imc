# `imc search` retrieval prompt (v2)

Grounded in measurements on `/root/imc` (25 labelled queries, `e2e/search_bench.py`)
and in commands verified to exist in the Rust binary (`python3 /tmp/verify_prompt.py`).

Two prompts live here because `imc search` is used in two different places.

---

## A. Prompt that gets shipped in `CLAUDE.md`

This is the one an agent reads before calling the tool. Rules and formats, no prose.

```markdown
<!-- imc:begin -->
## imc — code intelligence for this repo

Local AST + call-graph + vector index. Prefer it over blind Read/Grep to understand
code. Every command below is verified to exist in this binary.

**Find code**
- `imc search "<3-7 words>"` — answers from the indexed chunks with an LLM
  chunks. Measured on this repo: top-1 hit is right 52% of the time, top-5 is right 80%.
  **Always read the top 5, never stop at hit 1.**
- `imc search "<exact identifier>"` — for a known symbol this is exact, not fuzzy:
  case and `_`/camelCase are normalised on both sides. Prefer it when you know the name.
- `imc search "<q>" --json` — machine-readable hits, for when you post-process.
- `imc search "<q>" --no-vector` — BM25 only; use when no embedding endpoint is
  configured or when you want deterministic, offline results.

**Read less**
- `imc read <file> --symbols` — outline first, then read only the lines you need.
- `imc read <file> --method <name>` — one symbol's body. **Use the real name from
  `--symbols`, including any leading underscore** (`_blended_score`, not `blended_score`).
- `imc context <file>` — the whole feature around a file: component, deps, tests.

**Understand impact**
- `imc trace <symbol> --callers --depth 3` — blast radius. Run this *before* renaming
  or deleting anything. `imc callers <symbol>` / `imc blast <symbol>` are shorthands.
- `imc trace <symbol>` — what it calls (forward); `--both` walks both directions.
- `imc graph <symbol>` — definition site and kind.

**Recall past decisions**
- `imc memory-search "<topic>"` — conventions and decisions previously recorded
  for this repo. Empty is a valid answer.

**Rules**
1. Search before you edit; never invent an API, path, or line number.
2. Cite `path/to/file:L10-L25` when answering. Never cite a range you did not see.
3. Hits are ranked evidence, not proof — open the file before relying on it. A
   query whose content words mostly do not exist in the index now returns nothing
   (junk-query gate), so an empty result means "not in the index".
4. If search misses repeatedly, fall back to Read/Grep. An empty result means
   "not in the index", not "does not exist".
<!-- imc:end -->
```

### What changed from the Python original, and why

| Python prompt said | Reality in the Rust binary | Change |
|---|---|---|
| `imc trace <symbol> --both` | no `--both` flag then | added `-b/--both` (+ `-r/--callers`, `-a/--all`, `-d/--depth`) |
| `imc callers <symbol>` | no `callers` subcommand then | added `callers` / `callees` / `blast` |
| `imc-read <file> --method <symbol>` | command is `imc read` | corrected |
| `"how indexing works \| ast and memory"` (directive) | directives are not evaluated; they only clamp the limit | directive guidance removed |
| `"chunk count: \| 8 chunks or --limit N"` | `--limit` works, directive does not | kept `-l/--limit`, dropped directive claim |
| asserted nothing about hit quality | measured 52% / 80% | told the agent to read top-5 |
| silent on identifier matching | 6/6 exact after the boost fix | documented the exact-identifier path |
| silent on `--method` needing real names | `_blended_score` fails as `blended_score` | explicit underscore warning |

Two of the four "next command" suggestions in the original prompt would have sent
the agent to a command that does not exist. That is the main reason this rewrite
is not cosmetic.

---

## B. System prompt for LLM synthesis over retrieved chunks

`imc search` in the Rust port does **not** call an LLM — it only prints ranked hits.
So this prompt has no caller yet. It is kept because the retrieval-side contract
is already fixed and the synthesis side is the natural next step; it is what
`main/prompts/imc_search.md` did in Python.

```markdown
You are a precise codebase retrieval assistant.

Answer using ONLY the chunks in <codebase_context>. Never invent a file, symbol,
route, or line range.

## Output
### Answer
Direct answer with inline `path/to/file.py:L10-L25` citations.

### Evidence
- `path/to/file.py:L10-L25` — why this chunk matters.
- Omit chunks that did not contribute.

## Rules
- No greeting, no restating the question.
- Cite every factual claim. If a claim has no chunk, drop it.
- If the context does not cover the question, answer exactly:
  `Not covered by retrieved context.`
- If it partially covers it, say `Partially covered by retrieved context.` and
  state what is missing.
- Separate what the code does from what you recommend.
- Use fenced blocks only for code.

When the question asks for impact of a change, list the symbols that must be
updated together. When it asks how something works, give entry point → call chain
→ result, using only edges visible in the context.
```

### Why it is written this way

- **"Not covered by retrieved context."** A confident hallucination is worse than a
  refusal, and retrieval can be weak — a junk query used to return five
  confident-looking hits, so the evidence rule is stated as a rule, not taste.
- **"Omit chunks that did not contribute."** The context is assembled by BM25+RRF,
  which optimises ranking, not relevance to a question. Without this the model
  tends to narrate every chunk it was given, including the filler.
- **Impact section.** `imc trace --callers` already computes the blast radius
  precisely; asking for it in prose keeps the answer aligned with the graph.

---

## Open gaps this work exposed

1. `imc search "<q> | directive"` silently ignores the directive. Either evaluate
   it (needs an LLM call) or drop the syntax. Today it looks supported.
2. `imc read --method` fails on a near-miss name (`blended_score` vs `_blended_score`)
   with no suggestion of the real name. Fuzzy-matching the outline would fix it.
3. Code search has no minimum-score floor. Memory search has one (0.45); code
   search does not, which is why junk queries return five hits.