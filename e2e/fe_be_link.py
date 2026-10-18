"""Frontend -> backend linkage without AST trace.

Two routes, measured against each other:
  A. string match   extract fetch() paths from JSX, match FastAPI decorators. Free.
  B. cheap LLM      same evidence, asked to an LLM. Costs a few cents.

    python3 e2e/fe_be_link.py --component ui/src/components/MemoryView.jsx
"""

import argparse
import json
import re
import subprocess
from pathlib import Path

REPO = Path("/root/imc")
IMC = Path("/root/imc-rs/target/release/imc")

# `/memory/${id}` -> `/memory/`; `{projectId}` -> `{projectId}`
def normalise(path: str) -> str:
    """Canonical path: strip query, collapse slashes, and fold path params.

    Frontend writes `/memory/${id}`, the backend registers `/api/memory/{memory_id}`.
    Both must reduce to the same shape or every parameterised endpoint misses.
    """
    path = path.split("?")[0]
    path = re.sub(r"\$\{[^}]*\}", "{p}", path)
    path = re.sub(r"\{[^}]*\}", "{p}", path)
    path = re.sub(r"/+", "/", path)
    return path.rstrip("/") or "/"


# `API_URL.replace(/\/api$/, '')` — the suffix it strips is the prefix value.
API_PREFIX_RE = re.compile(
    r"API_URL\.replace\(\s*/\\?/?([A-Za-z0-9_-]+)\$?/\s*,")


def api_prefix(js: str, sibling_js: str = "") -> str:
    """Resolve `${API_URL}` to its value.

    The fetch sites are written as `${API_URL}/memory`, but the backend
    registers `/api/memory` — stripping the variable loses the `/api` segment and
    every match fails. The codebase states the value itself: App.jsx does
    `API_URL.replace(<slash-escaped>/api$/, '')` to drop the prefix, which is
    direct evidence that API_URL ends in `/api`.
    """
    for source in (js, sibling_js):
        m = API_PREFIX_RE.search(source)
        if m:
            val = m.group(1).rstrip("/")
            return "/" + val.lstrip("/")
    return ""


def fetch_paths(js: str, prefix: str = "") -> list[tuple[str, str]]:
    """Return [(raw_url, resolved_path)] for every fetch() call."""
    out = set()
    for m in re.finditer(r"""fetch\(\s*[`'"]([^`'"]+)[`'"]""", js):
        raw = m.group(1)
        if "${API_URL}" in raw or "API_URL" in raw:
            resolved = prefix + re.sub(r"^\$\{?API_URL\}?", "", raw)
        else:
            resolved = raw
        if resolved.startswith("/"):
            out.add((raw, normalise(resolved)))
    return sorted(out)


def backend_routes():
    """(path, method, file, line) for FastAPI routes.

    Decorator paths are relative to the module's `APIRouter(prefix=...)`:
    `git.py` registers `/status` under `prefix="/api/git"`, while `memory.py`
    has no prefix and writes `/api/memory` in full. Reading only the decorator
    misses every prefixed router.
    """
    routes = []
    for f in REPO.rglob("*.py"):
        if "/tests/" in str(f) or "/node_modules/" in str(f):
            continue
        try:
            text = f.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        pm = re.search(r'APIRouter\(\s*prefix\s*=\s*[\'"]([^\'"]+)[\'"]', text)
        router_prefix = (pm.group(1).rstrip("/") if pm else "")
        for m in re.finditer(
            r'@router\.(get|post|put|delete|patch)\(\s*[\'"]([^\'"]+)[\'"]', text
        ):
            raw = router_prefix + m.group(2) if m.group(2).startswith("/") else m.group(2)
            method = m.group(1).upper()
            path = normalise(raw)
            line = text[: m.start()].count("\n") + 1
            routes.append((path, method, str(f.relative_to(REPO)), line))
    return routes


def search(q, n=5):
    p = subprocess.run([str(IMC), "-C", str(REPO), "search", q, "-l", str(n), "--json"],
                       cwd=str(REPO), capture_output=True, text=True, timeout=120)
    return json.loads(p.stdout) if p.returncode == 0 else []


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--component", default="ui/src/components/MemoryView.jsx")
    args = ap.parse_args()

    js = (REPO / args.component).read_text(encoding="utf-8", errors="ignore")
    app_js = (REPO / "ui/src/App.jsx").read_text(encoding="utf-8", errors="ignore")
    prefix = api_prefix(js, app_js)
    fe = fetch_paths(js, prefix)
    print(f"=== {args.component} ===")
    print(f"resolved API prefix: {prefix!r}")
    print("fetch endpoints:")
    for raw, resolved in fe:
        print(f"    {raw:52} -> {resolved}")

    routes = backend_routes()
    print(f"\nbackend routes found: {len(routes)} decorators")

    print("\n=== A. string match (free, no LLM, no AST) ===")
    found = 0
    for raw, resolved in fe:
        hits = [(r[1], r[2], r[3], "exact") for r in routes if r[0] == resolved]
        if not hits:
            hits = [(r[1], r[2], r[3], "prefix")
                    for r in routes
                    if resolved.startswith(r[0] + "/") or r[0].startswith(resolved + "/")]
        if hits:
            found += 1
            for method, file, line, kind in hits[:3]:
                print(f"  OK   {resolved:26} [{kind}] [{method}] -> {file}:{line}")
        else:
            print(f"  --   {resolved:26} no backend handler")
    print(f"  => {found}/{len(fe)} endpoints linked")

    # did the search leg find anything useful?
    print("\n=== B. imc search evidence for the chain ===")
    for _, resolved in fe[:4]:
        got = search(resolved, 3)
        print(f"  {resolved!r} -> {[h['file'] for h in got]}")

    print("\n=== handlers by name search ===")
    for name in ["create_memory", "delete_memory", "prune_stale_memories"]:
        got = search(name, 3)
        print(f"  {name:24} -> {[h['file'] for h in got]}")


if __name__ == "__main__":
    main()