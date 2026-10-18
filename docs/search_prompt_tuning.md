# `imc search` prompt tuning — measured results

Harness: `e2e/prompt_eval.py`. Eight answerable queries with labels verified
against the index, one unanswerable query for the refusal check, retrieval frozen
to a cache so every prompt sees identical context, three runs per prompt.

Scoring was corrected twice during this work; both corrections mattered.

## Result

| prompt | grounded (of 8) | key files (of 13) | invalid citations |
|---|---|---|---|
| `main/prompts/imc_search.md` (baseline) | 5.3 | 7.3 | 4.3 |
| **candidate A — tuned** | **7.0** | **8.3** | 1.7 |
| candidate C — line-number strict | 6.3 | 8.0 | 1.3 |
| candidate B — minimal | 8 (n=1) | 7 (n=1) | 0 |

A wins grounded and key-file coverage; C wins invalid citations by 0.3, which is
inside noise at n=3. A and C are each clearly better than baseline: baseline's
invalid range is 3–5, A's is 0–3, and paired across runs A wins every pair on
both metrics.

Candidate A is the recommendation: see `docs/search_prompt_tuned.md`.

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