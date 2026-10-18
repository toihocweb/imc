# `imc search` prompt tuning — measured results

Harness: `e2e/prompt_eval.py`. Eight answerable queries with labels verified
against the index, one unanswerable query for the refusal check, retrieval frozen
to a cache so every prompt sees identical context, three runs per prompt.

Scoring was corrected twice during this work; both corrections mattered.

## Result

| prompt | grounded (of 8) | key files (of 13) | invalid citations |
|---|---|---|---|
| `main/prompts/imc_search.md` (baseline) | 5.3 | 7.3 | 4.3 |
| candidate A — tuned | 7.0 | 8.3 | 1.7 |
| candidate C — line-number strict | 6.3 | 8.0 | 1.3 |
| candidate B — minimal | 8 (n=1) | 7 (n=1) | 0 |
| **candidate D — A + selective read** | **8.0** | **8.3** | **0.0** |

Per-run, for the two that matter:

    A   grounded 8,7,6   key 9,8,8   invalid 0,2,3
    D   grounded 8,8,8   key 9,8,8   invalid 0,0,0

Candidate D dominates A on every metric and holds zero invalid citations across
all three runs. The difference is a `## If the answer needs more code` section
that reinstates the original prompt's "Next command" idea — go to
`imc read <file> --method <name>` instead of reading a 700-line file for one
method — but with commands that actually exist in this binary.

That block was dropped in candidate A along with the three commands it recommended
(`imc-read`, `imc callers`, `imc trace --both`), none of which exist. The idea was
right and the measurements say so: adding it back with real commands took
grounding from 7.0 to 8.0 and invalid citations from 1.7 to 0.0. A prompt with no
place to send the model left it to guess line ranges.

Candidate D is the recommendation: see `docs/search_prompt_tuned.md`.

## What the harness got wrong first

Two bugs made the first measurements look better than the truth.

1. **It checked only that a cited file exists on disk.** A model that named a
   real file it had never seen in context still scored as grounded. Fixing it to
   require the file in `<codebase_context>` dropped the baseline from 9/13 to
   7/13 — most of the initial "good" score was phantom.

2. **One label was wrong.** "what languages does the tree sitter parser support"
   was scored as a miss, but the retrieved chunks for that phrasing are about
   file filtering (`is_indexable_file`, `FileIngester`) and contain no language
   list. The model declining to cite `parser.py` was correct; the label was not.

Retrieval also runs against a live embedding endpoint, so two sweeps minutes
apart can see different chunks. Without freezing context, comparing two prompts
measures retrieval noise. That cache is why the final table is comparable.

## Where the ceiling is

Even the best prompt leaves 1.3–1.7 invalid citations per sweep of 8 questions,
and run-to-run variance is large: candidate C ranged 8/8 → 5/8 → 6/8 on the
same frozen context with temperature 0.

Prompt wording has stopped being the lever. The durable fix is to validate
citations after generation — reject any cited span not present in the context
that was sent, and return it for a second pass. That would take invalid
citations to zero deterministically, which no amount of prose can do.