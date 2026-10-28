#!/usr/bin/env python3
"""
Performance Benchmark for imc:
Measures execution time of:
1. Indexing & Manifest walk (imc index --force .)
2. Context gathering & WalkDir traversal (imc context crates/imc-find/src/context.rs)
3. Fallback symbol scan (imc read crates/imc-find/src/find.rs --method nonexistent_foo_symbol)
"""

import subprocess
import time
import sys
import json
from pathlib import Path

def run_bench(imc_bin, iters_index=3, iters_sub=5):
    results = {}
    
    # 1. Indexing
    times = []
    for _ in range(iters_index):
        t0 = time.perf_counter()
        r = subprocess.run([imc_bin, "index", "--force", "."], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        t1 = time.perf_counter()
        if r.returncode != 0:
            print(f"Error in index: {r.stderr}", file=sys.stderr)
            return None
        times.append((t1 - t0) * 1000)
    results["index_force_ms"] = {
        "avg": round(sum(times) / len(times), 2),
        "min": round(min(times), 2),
        "max": round(max(times), 2)
    }

    # 2. Context
    times = []
    for _ in range(iters_sub):
        t0 = time.perf_counter()
        r = subprocess.run([imc_bin, "context", "crates/imc-find/src/context.rs"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        t1 = time.perf_counter()
        if r.returncode != 0:
            print(f"Error in context: {r.stderr}", file=sys.stderr)
            return None
        times.append((t1 - t0) * 1000)
    results["context_ms"] = {
        "avg": round(sum(times) / len(times), 2),
        "min": round(min(times), 2),
        "max": round(max(times), 2)
    }

    # 3. Read Fallback Search
    times = []
    for _ in range(iters_sub):
        t0 = time.perf_counter()
        r = subprocess.run([imc_bin, "read", "crates/imc-find/src/find.rs", "--method", "nonexistent_foo_symbol"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        t1 = time.perf_counter()
        if r.returncode != 0:
            print(f"Error in read fallback: {r.stderr}", file=sys.stderr)
            return None
        times.append((t1 - t0) * 1000)
    results["read_fallback_ms"] = {
        "avg": round(sum(times) / len(times), 2),
        "min": round(min(times), 2),
        "max": round(max(times), 2)
    }

    return results

if __name__ == "__main__":
    imc_path = sys.argv[1] if len(sys.argv) > 1 else "/root/.local/bin/imc"
    print(f"Running benchmark with binary: {imc_path} ...")
    res = run_bench(imc_path)
    if res:
        print(json.dumps(res, indent=2))
