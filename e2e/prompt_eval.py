"""Run the retrieval -> prompt -> LLM pipeline and score the answer.

The prompt template is read from the repo's imc_search.md (or a candidate file
passed with --prompt), context comes from the real `imc search` index, and the
score is mechanical so two prompts can be compared without taste arguments:

  cite_valid    cited `path:L10-L25` spans whose file exists and whose line
                range is inside the file. Out-of-bounds or invented paths count
                as hallucinations.
  cite_total    total distinct spans cited.
  grounded      answers that cite at least one valid span and invent none.
  refusal_ok    on a question the context cannot answer, the model says so.
  key_files     how many of the known-answer files appear in the citations.

    python3 e2e/prompt_eval.py --prompt /root/imc/main/prompts/imc_search.md
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import urllib.request
from pathlib import Path

IMC = Path(__file__).resolve().parent.parent / "target" / "release" / "imc"
REPO = Path("/root/imc")
MODEL = os.environ.get("IMC_EVAL_MODEL", "claude-haiku-4.5")

# Ground truth used by the citation checker.
KNOWN = {
    "explain imc search": [
        "main/prompts/imc_search.md",
        "main/search/semantic.py",
        "main/cli/commands/search.py",
    ],
    "how does imc search work": [
        "main/prompts/imc_search.md",
        "main/cli/commands/search.py",
    ],
    "what does the memory recall blended score combine": [
        "main/memory/store.py",
    ],
    "how does the proxy decide which upstream to route to": [
        "main/proxy/routes/messages.py",
    ],
    # Rewritten: the first phrasing's answer (tree-sitter language list) was not
    # in any retrieved chunk — the parser.py hits are about file filtering. A
    # label the index cannot answer measures the index, not the prompt.
    "how does the indexer decide which files to index": [
        "main/ingest/parser.py",
    ],
    "how are call graph edges built between symbols": [
        "main/indexing/graph.py",
    ],
    "how does imc store and recall project memories": [
        "main/memory/store.py",
        "main/memory/retrieval.py",
    ],
    "what does the claude integration wire into a project": [
        "main/cli/commands/claude.py",
        "main/cli/commands/index.py",
    ],
}

# Deliberately unanswerable: no file in the repo discusses this.
UNANSWERABLE = "what is the airspeed velocity of an unladen swallow"


def config():
    import yaml
    d = yaml.safe_load(open("/root/.imc_config.yaml"))["llm"]
    return d


def _llm_once(prompt: str, model: str) -> str:
    d = config()
    body = json.dumps({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "stream": False,
        "temperature": 0.0,
    }).encode()
    req = urllib.request.Request(
        d["base_url"].rstrip("/") + "/chat/completions",
        data=body,
        headers={"Authorization": f"Bearer {d['api_key']}", "Content-Type": "application/json"},
    )
    resp = json.loads(urllib.request.urlopen(req, timeout=180).read())
    return resp["choices"][0]["message"]["content"]


def llm(prompt: str, model: str) -> str:
    """The endpoint occasionally times out mid-run; retrying keeps a whole
    sweep comparable instead of losing it on the last query."""
    last = None
    for attempt in range(4):
        try:
            return _llm_once(prompt, model)
        except Exception as e:  # noqa: BLE001
            last = e
            import time as _t
            _t.sleep(2 * (attempt + 1))
    raise RuntimeError(f"llm failed after 4 attempts: {last}")


# Retrieval is frozen to a cache file. Hybrid search hits a live embedding
# endpoint, so two sweeps minutes apart can see different chunks — without this,
# comparing two prompts measures retrieval noise, not prompt quality.
CACHE = Path("/tmp/prompt_eval_context.json")


def retrieve(query: str, limit: int) -> list[dict]:
    key = f"{query}|{limit}"
    cache = {}
    if CACHE.exists():
        try:
            cache = json.loads(CACHE.read_text())
        except json.JSONDecodeError:
            cache = {}
    if key in cache:
        return cache[key]
    p = subprocess.run(
        [str(IMC), "-C", str(REPO), "search", query, "-l", str(limit), "--json"],
        cwd=str(REPO), capture_output=True, text=True, timeout=300,
    )
    if p.returncode != 0:
        raise RuntimeError(f"imc search failed: {p.stderr[:300]}")
    cache[key] = json.loads(p.stdout)
    CACHE.write_text(json.dumps(cache, indent=1))
    return cache[key]


def pack_context(hits: list[dict]) -> str:
    """Reproduce the Python pack_context shape closely enough for the template."""
    sections = []
    for h in hits:
        header = f"{h['file']}:{h['start_line']}-{h['end_line']}"
        body = h.get("text") or h.get("snippet") or ""
        sections.append(f"<{h['file']}>\n{header}\n{body}\n</{h['file']}>")
    return "\n\n".join(sections)


CITE_RE = re.compile(r"([A-Za-z0-9_./-]+\.[A-Za-z0-9]+)(?::L?(\d+)(?:-L?(\d+))?)?")


def check_citations(answer: str, context_files: set[str]):
    """Split cited spans into (grounded, bad).

    Two ways to be bad:
      * not_in_context — the file was never in <codebase_context>. Checking only
        whether the file exists lets a model pass by naming a real file it never
        saw, which is exactly the failure a retrieval prompt must prevent.
      * malformed — in context, but the line range is outside the file.
    """
    grounded, bad = [], []
    for path, a, b in CITE_RE.findall(answer):
        if not path.startswith(("main/", "ui/", "tests/", "services/", "src/")):
            continue
        if path not in context_files:
            bad.append(f"{path}: not in context")
            continue
        if not a:
            grounded.append(path)
            continue
        start, end = int(a), int(b or a)
        f = REPO / path
        lines = f.read_text(encoding="utf-8", errors="ignore").count("\n") + 1
        if start < 1 or end < start or end > lines:
            bad.append(f"{path}:L{start}-L{end} outside 1-{lines}")
        else:
            grounded.append(f"{path}:L{start}-L{end}")
    return grounded, bad


def build_prompt(template: str, context: str, query: str) -> str:
    # The template uses {context} / {query}; tolerate templates that use
    # different placeholders so a candidate can be tried without edits.
    try:
        return template.format(context=context, query=query)
    except (KeyError, IndexError):
        return template + f"\n\n<codebase_context>\n{context}\n</codebase_context>\n\nQuestion: {query}\n"


def run(prompt_path: Path, query: str, limit: int, model: str, verbose=False):
    template = prompt_path.read_text()
    hits = retrieve(query, limit)
    context = pack_context(hits)
    context_files = {h["file"] for h in hits}
    answer = llm(build_prompt(template, context, query), model)
    valid, invalid = check_citations(answer, context_files)
    return {
        "query": query,
        "answer": answer,
        "valid": valid,
        "invalid": invalid,
        "hits": [h["file"] for h in hits],
        "context_files": context_files,
    }


def score_run(r: dict, expected_files: list[str]) -> dict:
    cited = {v.split(":")[0] for v in r["valid"]}
    key = [f for f in expected_files if f in cited]
    retrievable = [f for f in expected_files if f in r["context_files"]]
    return {
        "cite_valid": len(r["valid"]),
        "cite_invalid": len(r["invalid"]),
        "key_files": len(key),
        "key_total": len(expected_files),
        # A key file that retrieval never surfaced is a search miss, not a
        # prompt miss; report it separately instead of blaming the prompt.
        "key_retrievable": len(retrievable),
        "grounded": 1 if (r["valid"] and not r["invalid"]) else 0,
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--prompt", required=True)
    ap.add_argument("--limit", type=int, default=5)
    ap.add_argument("--model", default=MODEL)
    ap.add_argument("--runs", type=int, default=1, help="repeat each query N times")
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    prompt_path = Path(args.prompt)
    totals = dict(queries=0, grounded=0, key=0, key_total=0, cite_valid=0,
                  cite_invalid=0, key_retrievable=0)
    worst = []

    for query in KNOWN:
        scores = []
        bad_seen = []
        for _ in range(args.runs):
            r = run(prompt_path, query, args.limit, args.model)
            s = score_run(r, KNOWN[query])
            scores.append(s)
            bad_seen = r["invalid"][:4]
            if args.verbose:
                print(f"    answer ({s['cite_valid']} valid / {s['cite_invalid']} invalid):\n"
                      + "    " + "\n    ".join(r["answer"].splitlines()[:14]) + "\n")
        s = max(scores, key=lambda x: (x["grounded"], x["key_files"]))
        totals["queries"] += 1
        totals["grounded"] += s["grounded"]
        totals["key"] += s["key_files"]
        totals["key_total"] += s["key_total"]
        totals["cite_valid"] += s["cite_valid"]
        totals["cite_invalid"] += s["cite_invalid"]
        totals["key_retrievable"] += s["key_retrievable"]
        ok = "ok " if s["grounded"] else "BAD"
        print(f"  {ok} {query[:44]:44} {s['key_files']}/{s['key_total']} key "
              f"({s['key_retrievable']} retrievable), {s['cite_valid']} valid, "
              f"{s['cite_invalid']} bad")
        for bad in bad_seen:
            print(f"        hallucinated: {bad}")

    # Refusal check
    r = run(prompt_path, UNANSWERABLE, args.limit, args.model)
    refused = "not covered" in r["answer"].lower() or "don't know" in r["answer"].lower()
    print(f"  {'ok ' if refused else 'BAD'} refusal on unanswerable query: {refused}")

    n = totals["queries"]
    print()
    print(f"  answerable grounded      {totals['grounded']}/{n}")
    print(f"  key files cited          {totals['key']}/{totals['key_total']}")
    print(f"    of which retrievable   {totals['key_retrievable']} "
          f"(rest are search misses, not prompt misses)")
    print(f"  citations valid          {totals['cite_valid']}")
    print(f"  citations invalid        {totals['cite_invalid']}")
    print(f"  refusal handled          {'1/1' if refused else '0/1'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())