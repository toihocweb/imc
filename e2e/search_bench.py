#!/usr/bin/env python3
"""Search-quality benchmark for `imc search` against a real repository.

Each query names something that exists in the repo and the file(s) that should
rank for it. Reports top-1 / top-3 / top-5 hit rates separately for the BM25-only
leg and the hybrid (BM25 + vector) path, plus mean reciprocal rank, so a change
to ranking can be judged instead of guessed at.

    python3 e2e/search_bench.py --repo /root/imc --mode both
"""

from __future__ import annotations

import argparse
import json
import statistics
import subprocess
import sys
from pathlib import Path

IMC = Path(__file__).resolve().parent.parent / "target" / "release" / "imc"

# (query, acceptable files) — acceptable = any of these counts as a hit.
# Expected files were checked against the repo tree first: a benchmark whose
# answers do not exist measures the benchmark, not the search.
QUERIES = [
    ("cli entry point argument parsing", ["main/cli/main.py"]),
    ("tokenization and bm25 scoring", ["main/search/semantic.py", "main/indexing/semantic.py"]),
    ("blended score weights for memory recall", ["main/memory/store.py"]),
    ("memory retrieval relevance scoring", ["main/memory/retrieval.py", "main/memory/store.py"]),
    ("auto learning from failed tool calls", ["main/memory/auto_learning.py"]),
    ("redact secrets api keys before storing", ["main/memory/auto_learning.py", "main/memory/prompts.py"]),
    ("prune stale memories referencing deleted files", ["main/memory/store.py"]),
    ("feedback events utility damping", ["main/memory/events.py"]),
    ("llm extraction of memories from chat", ["main/memory/extractor.py", "main/memory/prompts.py"]),
    ("tree sitter parse symbols from source", ["main/ingest/parser.py"]),
    ("build call graph edges between symbols", ["main/indexing/graph.py"]),
    ("semantic chunking of files into index", ["main/indexing/semantic.py"]),
    ("knowledge base documents", ["main/indexing/kb.py", "main/api/routers/kb.py"]),
    ("streaming response conversion to anthropic format", ["main/proxy/routes/messages.py"]),
    ("provider model catalog and limits", ["main/proxy/lib/provider_model.py", "main/proxy/lib/models.py"]),
    ("request context and headers per request", ["main/proxy/lib/request_context.py"]),
    ("git status router endpoint", ["main/api/routers/git.py"]),
    ("project manager create and list projects", ["main/project/manager.py"]),
    ("mcp server tools registration", ["main/mcp/server.py"]),
    ("api server uvicorn startup", ["main/api/server.py"]),
    ("configuration settings loading and defaults", ["main/core/config.py"]),
    ("tracer initialization and session setup", ["main/core/tracer.py"]),
    ("context command collect relevant files", ["main/cli/commands/context.py"]),
    ("compact conversation history token limit", ["main/cli/commands/compact.py"]),
    ("index command incremental reindex", ["main/cli/commands/index.py"]),
]

JUNK = [
    "quantum banana teleport protocol",
    "kubernetes helm chart rollout strategy",
    "photosynthesis chlorophyll wavelength",
]


def check_answers_exist(repo):
    """Fail loudly if an expected file is absent — otherwise a benchmark quietly
    measures its own bad labels instead of search quality."""
    bad = []
    for q, acceptable in QUERIES:
        for f in acceptable:
            if not (repo / f).exists():
                bad.append((q, f))
    if bad:
        print("ERROR: these expected files do not exist — fix the labels first:")
        for q, f in bad:
            print(f"  {q}: {f}")
        sys.exit(2)


def run(repo, args, expect_rc=0):
    # -C is explicit: without it resolve_root prefers the globally active
    # project over cwd, so an unrelated project can answer the query.
    p = subprocess.run([str(IMC), "-C", str(repo), *args], cwd=str(repo),
                       capture_output=True, text=True, timeout=300)
    if p.returncode != expect_rc:
        raise RuntimeError(f"rc={p.returncode}: {p.stderr[:300]}")
    return p.stdout


def evaluate(repo, query, acceptable, limit=5):
    out = json.loads(run(repo, ["--json", "search", query, "-l", str(limit)]))
    files = [h["file"] for h in out]
    def rank_of(r):
        for i, f in enumerate(files, 1):
            if f in acceptable:
                return i
        return None
    return files, rank_of


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", default="/root/imc")
    ap.add_argument("--mode", default="both", choices=["both", "bm25", "hybrid"])
    args = ap.parse_args()
    repo = Path(args.repo)

    check_answers_exist(repo)

    stats = json.loads(run(repo, ["--json", "stats"]))
    print(f"repo: {repo}")
    print(f"indexed: {stats.get('files')} files, {stats.get('symbols')} symbols, "
          f"{stats.get('chunks')} chunks, {stats.get('edges', '?')} edges")

    modes = []
    if args.mode in ("both", "bm25"):
        modes.append(("BM25 only", ["--no-vector"]))
    if args.mode in ("both", "hybrid"):
        modes.append(("hybrid", []))

    for label, extra in modes:
        print(f"\n=== {label} ===")
        rr = []
        t1 = t3 = t5 = 0
        misses = []
        for q, acceptable in QUERIES:
            base = ["--json", "search", q, "-l", "5"]
            if extra:
                out = json.loads(run(repo, base + extra))
            else:
                out = json.loads(run(repo, base))
            files = [h["file"] for h in out]
            rank = None
            for i, f in enumerate(files, 1):
                if f in acceptable:
                    rank = i
                    break
            if rank == 1:
                t1 += 1
            if rank and rank <= 3:
                t3 += 1
            if rank and rank <= 5:
                t5 += 1
            else:
                misses.append((q, acceptable, files[:3]))
            if rank:
                rr.append(1.0 / rank)
            mark = "ok " if rank == 1 else ("~  " if rank else "MISS")
            print(f"  {mark} [{rank or '-':>2}] {q[:52]:52} -> {files[0] if files else '(none)'}")

        n = len(QUERIES)
        print(f"\n  top-1 {t1}/{n} ({t1/n:.0%})   top-3 {t3}/{n} ({t3/n:.0%})   "
              f"top-5 {t5}/{n} ({t5/n:.0%})   MRR {statistics.mean(rr):.3f}")
        if misses:
            print("\n  misses (want / got):")
            for q, acceptable, got in misses:
                print(f"    {q[:48]:48} want {acceptable} got {got}")

    print("\n=== junk queries (relevance floor) ===")
    for label, extra in modes:
        base = ["--json", "search"]
        if extra:
            base += extra
        for q in JUNK:
            out = json.loads(run(repo, base + [q, "-l", "5"]))
            status = "empty" if not out else f"{len(out)} hits ({out[0]['file']})"
            print(f"  {label:9} {q[:44]:44} -> {status}")
    return 0


if __name__ == "__main__":
    sys.exit(main())