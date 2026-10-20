#!/usr/bin/env python3
"""A larger, auto-generated search benchmark.

`search_bench.py` has 25 hand-written questions — enough to smoke out a broken
search, not enough to tell two chunking strategies apart (a 3-query swing on
n=25 is noise: McNemar's exact test on a 2-vs-3 flip gives p=1.0).

This builds the labelled set from the repository itself: every function with a
docstring becomes one question — the first sentence of the docstring is the
query, the file that defines it is the expected answer. The wording comes
from a human, not from the symbol name, so the question still has to be
matched against the indexed text.

    python3 e2e/search_bench_auto.py --repo /root/imc --limit 150

Reports top-1 / top-3 / top-5 hit rates and MRR, per mode, plus a paired
comparison when a second run's JSON is given:

    python3 e2e/search_bench_auto.py --repo /root/imc --save /tmp/after.json
    python3 e2e/search_bench_auto.py --repo /root/imc --compare /tmp/before.json
"""
from __future__ import annotations

import argparse
import ast
import json
import re
import statistics
import subprocess
import sys
from pathlib import Path

IMC = Path(__file__).resolve().parent.parent / "target" / "release" / "imc"

SKIP_DIRS = {".git", "__pycache__", "node_modules", ".venv", "venv", "dist", "build"}


def collect_questions(repo: Path, limit: int) -> list[tuple[str, str]]:
    """One question per documented function: the docstring's first sentence."""
    out: list[tuple[str, str]] = []
    for path in sorted(repo.rglob("*.py")):
        if any(part in SKIP_DIRS for part in path.parts):
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
            first = doc.strip().split("\n\n")[0]
            first = re.sub(r"\s+", " ", first).strip()
            # Keep it a question, not a paragraph.
            if len(first) < 25 or len(first) > 220:
                continue
            out.append((first, rel))
            if len(out) >= limit * 3:
                break
    # A deterministic spread over the repo, then cut to the limit.
    step = max(len(out) // limit, 1)
    picked = out[::step][:limit]
    return picked


def run(repo: Path, query: str, extra: list[str]) -> list[str]:
    args = [str(IMC), "-C", str(repo), "search", query, "--json", "-l", "5", *extra]
    try:
        r = subprocess.run(args, capture_output=True, text=True, timeout=60)
    except subprocess.TimeoutExpired:
        return []
    if r.returncode != 0:
        return []
    try:
        hits = json.loads(r.stdout)
    except json.JSONDecodeError:
        return []
    return [h.get("file", "") for h in hits if isinstance(h, dict)]


def evaluate(repo: Path, questions: list[tuple[str, str]], extra: list[str]) -> dict:
    ranks = []
    misses = []
    for query, expected in questions:
        files = run(repo, query, extra)
        rank = 0
        for i, f in enumerate(files, 1):
            if f == expected:
                rank = i
                break
        ranks.append(rank)
        if rank == 0:
            misses.append((query, expected, files[:3]))
    top1 = sum(1 for r in ranks if r == 1)
    top3 = sum(1 for r in ranks if 1 <= r <= 3)
    top5 = sum(1 for r in ranks if 1 <= r <= 5)
    return {
        "n": len(ranks),
        "top1": top1,
        "top3": top3,
        "top5": top5,
        "mrr": round(statistics.mean(1.0 / r if r else 0.0 for r in ranks), 4),
        "ranks": ranks,
        "misses": misses[:12],
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", default="/root/imc")
    ap.add_argument("--limit", type=int, default=150)
    ap.add_argument("--mode", default="both", choices=["both", "bm25", "hybrid"])
    ap.add_argument("--save", default="")
    ap.add_argument("--compare", default="")
    args = ap.parse_args()
    repo = Path(args.repo)
    questions = collect_questions(repo, args.limit)
    if not questions:
        print("no questions collected", file=sys.stderr)
        return 1
    print(f"repo: {repo}   questions: {len(questions)}")

    modes = []
    if args.mode in ("both", "bm25"):
        modes.append(("BM25 only", ["--no-vector"]))
    if args.mode in ("both", "hybrid"):
        modes.append(("hybrid", []))

    results = {}
    for label, extra in modes:
        res = evaluate(repo, questions, extra)
        results[label] = res
        print(
            f"  {label:10s} top1 {res['top1']}/{res['n']} ({100*res['top1']//res['n']}%)"
            f"  top3 {res['top3']}  top5 {res['top5']}"
            f"  MRR {res['mrr']}"
        )

    if args.save:
        Path(args.save).write_text(json.dumps(results, indent=2))
        print(f"saved → {args.save}")

    if args.compare:
        before = json.loads(Path(args.compare).read_text())
        for label in results:
            if label not in before:
                continue
            b, a = before[label], results[label]
            n = min(len(b["ranks"]), len(a["ranks"]))
            wins = losses = 0
            for i in range(n):
                hb = b["ranks"][i] == 1
                ha = a["ranks"][i] == 1
                if ha and not hb:
                    wins += 1
                elif hb and not ha:
                    losses += 1
            print(f"\n{label}: top1 {b['top1']} → {a['top1']}   MRR {b['mrr']} → {a['mrr']}")
            print(f"  flipped to HIT: {wins}   flipped to MISS: {losses}   (n={n})")
            from math import comb

            d = wins + losses
            p = 1.0
            if d:
                k = min(wins, losses)
                p = min(1.0, 2 * sum(comb(d, i) for i in range(k + 1)) / 2**d)
            print(f"  McNemar exact p = {p:.4f}  → "
                  f"{'không có bằng chứng khác biệt' if p > 0.05 else 'có bằng chứng khác biệt'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
