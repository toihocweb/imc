"""Run 10 natural-language questions through the full imc pipeline and write a
markdown report: query -> retrieved context -> LLM answer -> citation check.

    python3 e2e/qna_report.py --out docs/qna_report.md
"""

import argparse
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from prompt_eval import REPO, build_prompt, llm, pack_context, retrieve  # noqa: E402

QUERIES = [
    "how graph works",
    "how search works",
    "how memory works",
    "how the proxy works",
    "how indexing works",
    "how trace works",
    "how config works",
    "how context works",
    "how claude integration works",
    "how compact works",
]

CITE_RE = re.compile(r"([A-Za-z0-9_./-]+\.[A-Za-z0-9]+)(?::L?(\d+)(?:-L?(\d+))?)?")


def check(answer, context_files, context_text=""):
    """Grounded if the path was retrieved as a file, or named *inside* a chunk.

    A module referenced by an import line in a retrieved chunk is legitimate
    evidence even though no chunk of that file was retrieved — requiring the
    latter produced a false positive on `main/cli/commands/index.py`, which
    `claude.py:21-28` names directly in `from ... import`.
    """
    cited, bad = [], []
    for path, a, b in CITE_RE.findall(answer):
        if not path.startswith(("main/", "ui/", "tests/", "services/", "src/")):
            continue
        if path in cited:
            continue
        cited.append(path)
        if path not in context_files and path not in context_text:
            bad.append(f"{path} (not in context)")
        elif path in context_files and a:
            n = (REPO / path).read_text(encoding="utf-8", errors="ignore").count("\n") + 1
            if int(a) < 1 or int(b or a) > n:
                bad.append(f"{path}:L{a}-{b} (file has {n} lines)")
    return cited, bad


def first_answer_block(ans: str, limit: int = 700) -> str:
    """Keep the report readable: the Answer section, trimmed."""
    m = re.search(r"###\s*Answer(.*?)(###\s*Evidence|\Z)", ans, re.S)
    body = m.group(1).strip() if m else ans.strip()
    return body[:limit] + ("…" if len(body) > limit else "")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="/root/imc-rs/docs/qna_report.md")
    ap.add_argument("--model", default="claude-haiku-4.5")
    ap.add_argument("--limit", type=int, default=5)
    args = ap.parse_args()

    template = (Path(__file__).resolve().parent.parent / "docs" /
                "search_prompt_tuned.md").read_text()
    template = template[template.index("You are a precise"):]

    out = [
        "# `imc search` + LLM — end-to-end answers for 10 questions",
        "",
        f"Model `{args.model}`, retrieval top-{args.limit} from the real index of "
        "`/root/imc` (183 files), prompt = `docs/search_prompt_tuned.md`.",
        "",
        "Each entry shows what retrieval actually returned, then the model's answer,",
        "then a mechanical check of every cited path against the context that was sent.",
        "",
    ]

    totals = dict(cited=0, bad=0, partial=0)
    for q in QUERIES:
        hits = retrieve(q, args.limit)
        files = {h["file"] for h in hits}
        ctx = pack_context(hits)
        ans = llm(build_prompt(template, ctx, q), args.model)
        cited, bad = check(ans, files, ctx)
        totals["cited"] += len(cited)
        totals["bad"] += len(bad)
        if ans.strip().lower().startswith(("partially", "not covered")):
            totals["partial"] += 1

        print(f"  {q}: {len(cited)} cited, {len(bad)} bad", flush=True)

        out += [f"## `{q}`", ""]
        out.append("**Retrieved**")
        out.append("")
        for i, h in enumerate(hits, 1):
            out.append(f"{i}. `{h['file']}:{h['start_line']}-{h['end_line']}` "
                       f"— {h['score']:.4f}")
        out += ["", "**Answer**", ""]
        out.append(first_answer_block(ans))
        out += ["", "**Citations**", ""]
        out.append(f"- cited: {', '.join('`' + c + '`' for c in cited) or 'none'}")
        out.append(f"- invalid: {', '.join(bad) or 'none'}")
        out.append("")

    out += ["## Totals", "",
            f"- distinct paths cited: {totals['cited']}",
            f"- invalid citations: {totals['bad']}",
            f"- answers that opened by declaring partial coverage: {totals['partial']}/"
            f"{len(QUERIES)}", ""]

    Path(args.out).write_text("\n".join(out))
    print(f"\nwrote {args.out}")
    print(f"  cited {totals['cited']}, invalid {totals['bad']}, "
          f"partial {totals['partial']}/{len(QUERIES)}")


if __name__ == "__main__":
    main()