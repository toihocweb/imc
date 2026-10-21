# `imc search` retrieval prompt (v2)

Grounded in measurements on `/root/imc` (25 labelled queries, `e2e/search_bench.py`)
and in commands verified to exist in the Rust binary (`python3 /tmp/verify_prompt.py`).

`imc search` is used in two different places, so two prompts live here: the one the
agent reads (section A — the shipped text now lives with the code), and the one the
model reads when it synthesises an answer over the retrieved chunks (section B).

---

## A. The prompt the agent reads

The shipped text lives in one place: `imc_rule_md()` in
`crates/imc-cli/src/claude.rs`, written to `.claude/rules/imc-rule.md` by
`imc setup`. The copy in `docs/imc-rule.md` is the same text, and a test
compares the two so they cannot drift.

It is deliberately not written into `CLAUDE.md`: that file is the user's own,
and the rule file is what the agent reads for this project.

What the text has to do:

1. Open with a table: "you want X → run Y". The agent has one look at it.
2. Then the commands, each with the flags that change the answer, and no flag
   that does not.
3. Say which of two ways to get a thing is the right one (`imc trace` over
   `imc graph`, `imc pack` over stacking `search` + `read` + `context`).
4. Say what not to do: invent a path, cite a line you did not read, stop at hit
   one.
5. Stay about the tool. No setup section: the agent does not install things.
   No MCP: the rule is about the CLI the agent has, and a second surface is a
   second thing to keep in sync.

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