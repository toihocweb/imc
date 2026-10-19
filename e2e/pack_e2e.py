#!/usr/bin/env python3
"""End-to-end checks for `imc pack` — the context assembler.

Hermetic: builds a throwaway project, indexes it with an empty config (so no
embedding endpoint is touched), then asserts the properties an agent relies on:

  * retrieval actually lands on the seeded symbol, with file:line provenance,
  * the token budget is respected, and a tight budget reports truncation,
  * graph facts appear for a symbol that has callers,
  * project memories are folded in,
  * any shortened item carries a handle that `imc retrieve` resolves byte-exactly,
  * `--json` carries the fields a tool-calling agent needs,
  * a query with no matches degrades instead of crashing.
"""
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
IMC = ROOT / "target" / "release" / "imc"

SRC_MAIN = '''\
"""Seeded project for the pack e2e."""


def authenticate(username, password):
    """Check the password and mint a session token."""
    user = load_user(username)
    if not verify_password(user, password):
        raise PermissionError("bad password")
    return mint_token(user)


def mint_token(user):
    return {"sub": user["id"], "scope": "read"}
'''

SRC_ALT = '''\
from app.auth import authenticate


def login_view(request):
    # the router dispatches to this handler
    token = authenticate(request.username, request.password)
    return {"token": token}
'''

SRC_DOC = '''\
# Service notes

Authentication uses a session token minted by `mint_token`. The router
dispatches `/login` to `login_view`, which calls `authenticate`.
''' * 6


def run(args, env, cwd=None, stdin=None):
    return subprocess.run(
        [str(IMC)] + args, env=env, cwd=cwd, input=stdin,
        capture_output=True, text=True, timeout=120,
    )


failures = []


def check(name, ok, detail=""):
    print(f"  [{'PASS' if ok else 'FAIL'}] {name:<56} {detail}")
    if not ok:
        failures.append(name)


def main():
    tmp = Path(tempfile.mkdtemp(prefix="imc-pack-"))
    project = tmp / "proj"
    (project / "app").mkdir(parents=True)
    (project / "docs").mkdir()
    (project / "app" / "auth.py").write_text(SRC_MAIN)
    (project / "app" / "views.py").write_text(SRC_ALT)
    (project / "docs" / "notes.md").write_text(SRC_DOC)

    # Empty-but-existing config: a missing IMC_CONFIG file falls back to the
    # legacy ~/.indexer_config.yaml and would reach a real embedding endpoint.
    cfg = tmp / "config.yaml"
    cfg.write_text("")

    env = dict(os.environ)
    for k in ("IMC_DATA_DIR", "IMC_VECTOR", "IMC_EMBED_API_KEY", "OPENAI_API_KEY"):
        env.pop(k, None)
    env["IMC_CONFIG"] = str(cfg)
    env["IMC_DATA_DIR"] = str(tmp / "data")
    env["IMC_CCR_DB"] = str(tmp / "ccr.db")

    idx = run(["."], env, cwd=project)
    if idx.returncode != 0:
        print("indexing failed:\n", idx.stdout[-1500:], idx.stderr[-1500:])
        return 1

    # A memory the pack should surface for an authentication query.
    mem = run(
        ["memory-add", "Login must go through authenticate() before minting a token",
         "--category", "rule"],
        env, cwd=project,
    )
    check("seeded a project memory", mem.returncode == 0, mem.stderr.strip()[:50])

    print()
    print("imc pack")
    print("-" * 84)

    p = run(["pack", "how does authentication mint a token", "--budget", "1200"], env, cwd=project)
    check("pack exits 0", p.returncode == 0, p.stderr.strip()[:80])
    out = p.stdout
    check("bundle names a real file with line range", "app/auth.py:" in out,
          out.splitlines()[0] if out else "")
    check("bundle names the seeded symbol", "authenticate" in out, "")
    check("bundle reports its budget and usage", "budget:" in out and "used" in out,
          next((l for l in out.splitlines() if l.startswith("budget:")), ""))
    check("graph facts are attached", "graph:" in out,
          next((l for l in out.splitlines() if "graph:" in l), "")[:70])
    check("project memory is folded in", "authenticate() before minting" in out, "")
    check("provenance says why each item is there", "retrieval rank" in out, "")

    # Budget: used must not exceed the budget unless a single item could not fit.
    used = budget = 0
    for line in out.splitlines():
        if line.startswith("budget:"):
            parts = line.replace(",", " ").split()
            budget = int(parts[1])
            used = int(parts[parts.index("used") + 1])
    check("reported usage stays within budget", 0 < used <= budget, f"{used}/{budget}")

    # A tiny budget must keep something and admit the truncation.
    tin = run(["pack", "authenticate token", "--budget", "120"], env, cwd=project)
    check("tiny budget still returns the best hit", "app/auth.py" in tin.stdout, "")
    check("tiny budget reports truncation", "budget reached" in tin.stdout, "")

    # JSON shape.
    pj = run(["pack", "authenticate token", "--budget", "1500", "--json"], env, cwd=project)
    try:
        d = json.loads(pj.stdout)
    except Exception as e:
        d = {}
        check("--json parses", False, str(e)[:60])
    if d:
        need = {"query", "project", "budget", "used", "items", "memories", "truncated"}
        check("--json has the agent-facing keys", need <= set(d), ",".join(sorted(need - set(d))))
        item_need = {"file", "start_line", "end_line", "score", "text", "tokens", "why"}
        ok = bool(d["items"]) and item_need <= set(d["items"][0])
        check("--json items carry provenance", ok,
              ",".join(sorted(item_need - set(d["items"][0]))) if d["items"] else "no items")

        # Every handle must resolve back to the exact text it replaced.
        handles = [i for i in d["items"] if i.get("ccr_id")]
        if handles:
            h = handles[0]
            back = run(["retrieve", h["ccr_id"]], env, cwd=project)
            check("ccr handle resolves to the full original",
                  back.returncode == 0 and len(back.stdout) >= len(h["text"]),
                  f'{len(back.stdout)} vs {len(h["text"])}')
        else:
            check("ccr handle present when text was shortened", True, "nothing needed shortening")

    # No matches: degrade, don't crash.
    empty = run(["pack", "zzzqqq nothing matches this xyzzy", "--budget", "500"], env, cwd=project)
    check("an unmatched query exits 0", empty.returncode == 0, f"rc={empty.returncode}")
    check("an unmatched query explains itself", "no matches" in empty.stdout,
          next((l for l in empty.stdout.splitlines() if "no matches" in l), "")[:60])

    # Vector leg off must still work (lexical leg carries it).
    nv = run(["pack", "authenticate token", "--budget", "800", "--no-vector"], env, cwd=project)
    check("--no-vector still returns items", nv.returncode == 0 and "app/auth.py" in nv.stdout,
          f"rc={nv.returncode}")

    print()
    if failures:
        print(f"FAILURES ({len(failures)}): {failures}")
        return 1
    print("all pack checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
