#!/usr/bin/env python3
"""Compression benchmark on real repo fixtures.

Measures four things per content kind:
  1. token savings (bytes/4 estimate, same estimator the CLI uses)
  2. signal retention — do the lines/items that matter survive?
  3. never-inflate — output is never bigger than input
  4. round-trip — the compressed output still parses (JSON) / keeps its markers

Fixtures are real files from this repo, not synthetic strings.
"""
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
IMC = ROOT / "target" / "release" / "imc"
FIX = ROOT / "e2e" / "fixtures"


KIND_SLUG = {
    "JsonArray": "json_array",
    "JsonObject": "json_object",
    "SourceCode": "source_code",
    "BuildLog": "build_log",
    "GitDiff": "git_diff",
    "PlainText": "plain_text",
    "Unknown": "unknown",
}


def imc_compress(text: str) -> dict:
    proc = subprocess.run(
        [str(IMC), "compress"], input=text, capture_output=True, text=True, timeout=120
    )
    if proc.returncode != 0:
        raise RuntimeError(f"imc compress failed: {proc.stderr[:300]}")
    return json.loads(proc.stdout)


def est_tokens(text: str) -> int:
    return max(1, len(text.encode()) // 4)


def load(name: str) -> str:
    p = FIX / name
    if not p.exists():
        return ""
    return p.read_text(errors="replace")


def main() -> int:
    json_array = Path("/tmp/srch.json").read_text() if Path("/tmp/srch.json").exists() else "[]"
    cases = [
        ("json_array (real imc search --json, 20 hits)", json_array, "json_array"),
        ("source_code (imc-search/src/lib.rs)", load("src_imc_search.rs"), "source_code"),
        ("build_log (real cargo build output)", load("build_log_real.txt"), "build_log"),
        ("git_diff (real commit e1001a3)", load("git_diff_real.txt"), "git_diff"),
    ]

    print("=" * 96)
    print("imc compression — real fixtures from this repo")
    print("=" * 96)
    print(f"{'fixture':<46} {'in':>8} {'out':>8} {'saved':>7} {'ratio':>6} {'kind':>12}")
    print("-" * 96)

    rows = []
    failures = []
    for label, content, expect_kind in cases:
        if not content:
            failures.append(f"{label}: fixture missing")
            continue
        r = imc_compress(content)
        tin = est_tokens(content)
        tout = est_tokens(r["compressed"])
        saved = tin - tout
        pct = saved / tin * 100 if tin else 0.0
        rows.append((label, tin, tout, saved, pct, r["kind"], content, r))

        kind_ok = KIND_SLUG.get(r["kind"], r["kind"]) == expect_kind
        if not kind_ok:
            failures.append(f"{label}: kind={r['kind']} expected {expect_kind}")
        if len(r["compressed"]) > len(content):
            failures.append(f"{label}: output LARGER than input (inflate bug)")
        print(
            f"{label:<46} {tin:>8} {tout:>8} {saved:>7} {pct:>5.1f}% {r['kind']:>12}"
        )

    print("-" * 96)
    if rows:
        avg = sum(r[4] for r in rows) / len(rows)
        total_in = sum(r[1] for r in rows)
        total_out = sum(r[2] for r in rows)
        overall = (total_in - total_out) / total_in * 100
        print(f"{'average of per-case savings':<46} {'':>8} {'':>8} {'':>7} {avg:>5.1f}%")
        print(f"{'aggregate (all tokens)':<46} {total_in:>8} {total_out:>8} {total_in-total_out:>7} {overall:>5.1f}%")

    # ---- signal retention checks -------------------------------------------
    print()
    print("signal retention")
    print("-" * 96)
    checks = []

    # build log: every ERROR line must survive
    blog = load("build_log_real.txt")
    if blog:
        r = imc_compress(blog)
        errs_in = [l for l in blog.splitlines() if "ERROR" in l or "error[" in l]
        errs_out = [l for l in r["compressed"].splitlines() if "ERROR" in l or "error[" in l]
        ok = len(errs_out) == len(errs_in)
        checks.append(("build_log keeps every ERROR line", ok, f"{len(errs_out)}/{len(errs_in)}"))

    # git diff: every +/- line must survive
    diff = load("git_diff_real.txt")
    if diff:
        r = imc_compress(diff)
        changed_in = [l for l in diff.splitlines() if (l.startswith("+") or l.startswith("-"))
                      and not l.startswith("+++") and not l.startswith("---")]
        changed_out = [l for l in r["compressed"].splitlines() if (l.startswith("+") or l.startswith("-"))
                       and not l.startswith("+++") and not l.startswith("---")]
        ok = len(changed_out) == len(changed_in)
        checks.append(("git_diff keeps every changed line", ok, f"{len(changed_out)}/{len(changed_in)}"))

    # json: output must still parse as JSON
    if json_array != "[]":
        r = imc_compress(json_array)
        try:
            parsed = json.loads(r["compressed"])
            checks.append(("json_array output is valid JSON", isinstance(parsed, list), f"{len(parsed)} items"))
        except Exception as e:
            checks.append(("json_array output is valid JSON", False, str(e)[:60]))

    # source code: every distinct code line survives (comments may be stripped)
    src = load("src_imc_search.rs")
    if src:
        r = imc_compress(src)

        def code_only(line: str) -> str:
            # Mirror the compressor's contract: a trailing `//` comment may go,
            # the code before it may not.
            s = line.strip()
            if s.startswith("//"):
                return ""
            idx = s.find(" //")
            return s[:idx].strip() if idx > 0 else s

        code_in = [c for c in (code_only(l) for l in src.splitlines()) if c]
        code_out = set(c for c in (code_only(l) for l in r["compressed"].splitlines()) if c)
        missing = [l for l in set(code_in) if l not in code_out]
        ok = len(missing) == 0
        checks.append(("source_code keeps every distinct code line", ok, f"{len(missing)} missing"))
        for m in missing[:3]:
            print(f"      missing: {m[:100]}")

    for name, ok, detail in checks:
        print(f"  [{'PASS' if ok else 'FAIL'}] {name:<46} {detail}")
        if not ok:
            failures.append(name)
    retention_checks = len(checks)

    # ---- CCR CLI round-trip ------------------------------------------------
    print()
    print("CCR CLI (compress --store -> retrieve -> ccr stats/list)")
    print("-" * 96)
    import os
    import tempfile

    ccr_dir = tempfile.mkdtemp(prefix="imc-ccr-")
    env = dict(os.environ, IMC_CCR_DB=os.path.join(ccr_dir, "test.db"))
    sample = load("build_log_real.txt") or "x"

    stored = subprocess.run(
        [str(IMC), "compress", "--store", "--raw"],
        input=sample, capture_output=True, text=True, env=env, timeout=60,
    )
    checks.append(
        ("imc compress --store exits 0", stored.returncode == 0, stored.stderr[:60].strip())
    )

    listing = subprocess.run(
        [str(IMC), "ccr", "list", "--json"], capture_output=True, text=True, env=env, timeout=60,
    )
    try:
        rows = json.loads(listing.stdout)
        rid = rows[0]["id"] if rows else ""
    except Exception:
        rid = ""
    checks.append(("imc ccr list sees the record", bool(rid), rid[:16]))

    if rid:
        back = subprocess.run(
            [str(IMC), "retrieve", rid], capture_output=True, text=True, env=env, timeout=60,
        )
        checks.append(
            ("imc retrieve returns the exact original",
             back.returncode == 0 and back.stdout == sample,
             f"{len(back.stdout)} vs {len(sample)}"),
        )

    stats = subprocess.run(
        [str(IMC), "ccr", "stats", "--json"], capture_output=True, text=True, env=env, timeout=60,
    )
    try:
        s = json.loads(stats.stdout)
        ok = s["records"] >= 1 and s["saved_tokens"] == s["original_tokens"] - s["compressed_tokens"]
    except Exception:
        ok = False
    checks.append(("imc ccr stats reconciles", ok, ""))

    missing = subprocess.run(
        [str(IMC), "retrieve", "deadbeefdeadbeef"], capture_output=True, text=True, env=env, timeout=60,
    )
    checks.append(("imc retrieve on an unknown id exits non-zero", missing.returncode != 0, f"rc={missing.returncode}"))

    for name, ok, detail in checks[retention_checks:]:
        print(f"  [{'PASS' if ok else 'FAIL'}] {name:<46} {detail}")
        if not ok:
            failures.append(name)

    # ---- CCR round-trip ----------------------------------------------------
    print()
    print("CCR round-trip (compress -> store -> retrieve original)")
    print("-" * 96)
    rt = subprocess.run(
        ["bash", "-lc", f"""
set -e
export IMC_DATA_DIR=/tmp/ccr_bench_data
rm -rf /tmp/ccr_bench_data
DB=/tmp/ccr_bench_data/bench.db
mkdir -p /tmp/ccr_bench_data
""" ],
        capture_output=True, text=True,
    )
    # Round-trip is covered by the Rust unit tests (ccr_store::tests); here we
    # only assert the CLI gives us back the exact original it was given.
    sample = (load("build_log_real.txt") or "x")[:4000]
    r = imc_compress(sample)
    identical = r["original"] == sample
    print(f"  [{'PASS' if identical else 'FAIL'}] CLI echoes the exact original ({len(sample)} bytes)")
    if not identical:
        failures.append("original echo mismatch")

    print()
    if failures:
        print(f"FAILURES ({len(failures)}):")
        for f in failures:
            print(f"  - {f}")
        return 1
    print("all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
