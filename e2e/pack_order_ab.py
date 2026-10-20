#!/usr/bin/env python3
"""A/B: does the strongest item belong at the end of a packed bundle?

The bundle an agent reads is a prompt. Two results say a model's attention is
weakest in the middle of a long context and strongest at the end, which is
where the model continues from ("lost in the middle", Liu et al. 2023). That
argues for putting the best match last.

This measures it instead of believing it:

  for every labelled question (the docstring of a documented function, with
  the file it comes from as the ground truth)
    1. build a pack with `imc pack --json`
    2. render the same items twice — strongest first, strongest last
    3. ask the model to answer using only that context
    4. check whether the answer cites the expected file

The retrieval is identical in both arms, so the only difference is the order.
A paired McNemar test on the two arms says whether the difference is real.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import statistics
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

IMC = Path("/root/imc-rs/target/release/imc")
REPO = Path("/root/imc")
CACHE = Path("/tmp/pack_order_ab_cache.json")


def config() -> dict:
    import yaml

    d = yaml.safe_load(open("/root/.imc_config.yaml"))["llm"]
    return d


def llm(prompt: str, model: str) -> str:
    d = config()
    body = json.dumps(
        {
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "stream": False,
            "temperature": 0.0,
        }
    ).encode()
    req = urllib.request.Request(
        d["base_url"].rstrip("/") + "/chat/completions",
        data=body,
        headers={"Authorization": f"Bearer {d['api_key']}", "Content-Type": "application/json"},
    )
    resp = json.loads(urllib.request.urlopen(req, timeout=180).read())
    return resp["choices"][0]["message"]["content"]


def collect_questions(repo: Path, limit: int) -> list[tuple[str, str]]:
    """One question per documented function: the docstring's first sentence."""
    import ast

    out: list[tuple[str, str]] = []
    skip = {".git", "node_modules", "__pycache__", ".venv", "venv", "build", "dist"}
    for path in sorted(repo.rglob("*.py")):
        if any(part in skip for part in path.parts):
            continue
        try:
            tree = ast.parse(path.read_text(encoding="utf-8", errors="replace"))
        except (SyntaxError, OSError):
            continue
        rel = str(path.relative_to(repo))
        for node in ast.walk(tree):
            if not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
                continue
            doc = ast.get_docstring(node)
            if not doc:
                continue
            first = re.sub(r"\s+", " ", doc.strip().split("\n\n")[0]).strip()
            if len(first) < 25 or len(first) > 220:
                continue
            out.append((first, rel))
            if len(out) >= limit * 3:
                break
    step = max(1, len(out) // limit)
    return out[::step][:limit]


def pack_items(query: str, budget: int) -> list[dict]:
    p = subprocess.run(
        [str(IMC), "-C", str(REPO), "pack", query, "--json", "--budget", str(budget)],
        cwd=str(REPO),
        capture_output=True,
        text=True,
        timeout=300,
    )
    if p.returncode != 0:
        raise RuntimeError(f"imc pack failed: {p.stderr[:300]}")
    data = json.loads(p.stdout)
    return data.get("items", [])


def render(items: list[dict]) -> str:
    parts = []
    for it in items:
        head = f"--- {it['file']}:{it.get('start_line')}-{it.get('end_line')}"
        if it.get("symbol"):
            head += f" ({it['symbol']})"
        parts.append(f"{head} ---\n{it.get('text', '')}")
    return "\n\n".join(parts)


def cites(answer: str, expected: str) -> bool:
    """Did the answer point at the file the question came from?"""
    if not expected:
        return False
    base = os.path.basename(expected)
    if base in answer:
        return True
    return expected in answer


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--limit", type=int, default=60)
    ap.add_argument("--budget", type=int, default=3000)
    ap.add_argument("--model", default="claude-haiku-4.5")
    ap.add_argument("--save", default="/tmp/pack_order_ab.json")
    args = ap.parse_args()

    questions = collect_questions(REPO, args.limit)
    print(f"repo: {REPO}  questions: {len(questions)}  budget: {args.budget}", flush=True)

    cache = {}
    if CACHE.exists():
        try:
            cache = json.loads(CACHE.read_text())
        except json.JSONDecodeError:
            cache = {}

    arms = {"strongest_first": [], "strongest_last": []}
    for n, (query, expected) in enumerate(questions, 1):
        key = f"{query}|{args.budget}"
        if key not in cache:
            try:
                cache[key] = pack_items(query, args.budget)
            except Exception as e:  # noqa: BLE001
                print(f"  pack failed: {e}", flush=True)
                cache[key] = []
            CACHE.write_text(json.dumps(cache))
        items = cache[key]
        if len(items) < 2:
            # A one-item bundle has no order to compare.
            continue

        first = render(items)
        last = render(list(reversed(items)))
        prompt_tpl = (
            "Use only the context below to answer the question. "
            "Cite the file:line you used.\n\n"
            "Context:\n{ctx}\n\n"
            "Question: {q}\n\n"
            "Answer:"
        )
        answers = {}
        for arm, ctx in (("strongest_first", first), ("strongest_last", last)):
            try:
                answers[arm] = llm(prompt_tpl.format(ctx=ctx, q=query), args.model)
            except Exception as e:  # noqa: BLE001
                print(f"  llm failed: {e}", flush=True)
                answers[arm] = ""
        for arm in arms:
            arms[arm].append(1 if cites(answers[arm], expected) else 0)
        print(
            f"[{n}/{len(questions)}] first={arms['strongest_first'][-1]} "
            f"last={arms['strongest_last'][-1]}  {query[:60]}",
            flush=True,
        )

    res = {}
    for arm, vals in arms.items():
        if not vals:
            continue
        res[arm] = {
            "n": len(vals),
            "cited": sum(vals),
            "acc": round(statistics.mean(vals), 4),
        }
    # Paired McNemar on the discordant pairs.
    a = arms["strongest_first"]
    b = arms["strongest_last"]
    b_only = sum(1 for x, y in zip(a, b) if x == 0 and y == 1)
    a_only = sum(1 for x, y in zip(a, b) if x == 1 and y == 0)
    res["discordant"] = {"first_only": a_only, "last_only": b_only}
    if a_only + b_only > 0:
        n = a_only + b_only
        # Two-sided exact binomial p under p=0.5.
        from math import comb

        k = min(a_only, b_only)
        p = min(1.0, 2 * sum(comb(n, i) for i in range(k + 1)) / (2**n))
        res["mcnemar_p"] = round(p, 4)
    else:
        res["mcnemar_p"] = None

    Path(args.save).write_text(json.dumps(res, indent=2))
    print(json.dumps(res, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
