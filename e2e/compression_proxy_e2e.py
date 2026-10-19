#!/usr/bin/env python3
"""End-to-end check of the proxy compression endpoints over real HTTP.

compress -> store -> retrieve must give back the exact original bytes, and
stats must account for every stored record.
"""
import json
import sys
import urllib.request

BASE = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:47821"


def post(path, payload):
    req = urllib.request.Request(
        BASE + path,
        data=json.dumps(payload).encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.loads(r.read())


def get(path):
    with urllib.request.urlopen(BASE + path, timeout=10) as r:
        return json.loads(r.read())


failures = []


def check(name, ok, detail=""):
    print(f"  [{'PASS' if ok else 'FAIL'}] {name:<52} {detail}")
    if not ok:
        failures.append(name)


big_json = json.dumps([{"tool": "read", "file": f"src/f{i}.rs", "content": "x" * 3000} for i in range(6)])
log = "\n".join([f"2026-10-19 10:00:{i:02d} INFO step {i} ok" for i in range(20)]
                + ["2026-10-19 10:00:20 ERROR connection refused"])

print("proxy compression endpoints")
print("-" * 80)

r1 = post("/v1/compress", {"content": big_json})
check("POST /v1/compress returns an id", bool(r1.get("id")), r1.get("id", "")[:16])
check("kind detected as json_array", r1.get("kind") == "json_array", r1.get("kind"))
check(
    "compressed smaller than original",
    r1["compressed_tokens"] < r1["original_tokens"],
    f'{r1["original_tokens"]} -> {r1["compressed_tokens"]}',
)

r2 = post("/v1/retrieve", {"id": r1["id"]})
check("POST /v1/retrieve returns the exact original", r2["original"] == big_json,
      f'{len(r2["original"])} vs {len(big_json)} bytes')
check("retrieve also returns the compressed form", r2["compressed"] == r1["compressed"])

r3 = post("/v1/compress", {"content": log})
check("build log compresses", r3["compressed_tokens"] < r3["original_tokens"],
      f'{r3["original_tokens"]} -> {r3["compressed_tokens"]}')
check("build log keeps the ERROR line", "connection refused" in r3["compressed"])

r4 = post("/v1/retrieve", {"id": "does-not-exist"})
check("unknown id returns empty, not an error", r4.get("original") == "", repr(r4.get("original"))[:20])

stats = get("/v1/stats")
check("GET /v1/stats counts both records", stats["total_originals"] >= 2, stats["total_originals"])
check("stats totals are consistent",
      stats["total_saved_tokens"] == stats["total_original_tokens"] - stats["total_compressed_tokens"])
check("stats ratio in range", 0.0 < stats["compression_ratio"] <= 1.0, f'{stats["compression_ratio"]:.3f}')

print()
if failures:
    print(f"FAILURES: {failures}")
    sys.exit(1)
print("all endpoint checks passed")
