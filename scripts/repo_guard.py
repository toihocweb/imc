#!/usr/bin/env python3
"""Keep the imc repository clean.

The imc repo is a *tool* repository: it must never carry anything that belongs
to a real project someone is working on -- local paths, project names, captured
prompts, databases, or API keys.

This script runs as a git hook. It inspects the staged files and refuses the
commit when it finds:

  * a path that points at a home directory (`/Users/...`, `/home/<name>/...`
    for a name that is not a generic placeholder),
  * a project name listed in `scripts/repo_guard.deny` (tracked, shared) or
    `scripts/repo_guard.local.deny` (personal, not tracked -- the repository is
    public, so personal project names must not enter its history),
  * a file that should never be tracked (`.env`, `*.db`, captured prompts,
    locally generated Claude settings, ...),
  * something that looks like a real API key.

Usage:
    scripts/repo_guard.py                 # check the staged changes
    scripts/repo_guard.py --all           # check every tracked file
    scripts/repo_guard.py --message FILE  # check a commit message

Install it as a hook with: scripts/install_hooks.sh
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DENY_FILE = REPO / "scripts" / "repo_guard.deny"
# Personal, untracked list of project names.
LOCAL_DENY_FILE = REPO / "scripts" / "repo_guard.local.deny"
ALLOW_FILE = REPO / "scripts" / "repo_guard.allow"

# Files that must never be tracked.
FORBIDDEN_PATHS = re.compile(
    r"(^|/)(\.env(\..*)?|.*\.db|.*\.sqlite3?)$"
    r"|(^|/)\.claude/settings\.local\.json$"
    r"|^captures?/"
    r"|(^|/)memory_dump"
    r"|(^|/)\.imc-data/"
)

# A home directory that belongs to somebody. `/home/dev/...` and `/home/you/...`
# are generic placeholders used in the docs and tests, so they are allowed.
HOME_PATH = re.compile(r"/Users/[^/\s\"']+/")
LINUX_HOME = re.compile(r"/home/(?!dev/|you/|user/|runner/)[^/\s\"']+/")

# Key shapes: a long run of key characters behind a known prefix. The token
# may not contain a dot, so masked placeholders ("sk-abc...mnop") and test
# fixtures do not trip it.
SECRET = re.compile(
    r"^(?:sk[-_]|xai[-_]|ghp[_]|gho[_])[A-Za-z0-9_-]{16,}$|^AIza[A-Za-z0-9_-]{30,}$"
)
ASSIGNED_KEY = re.compile(
    r"(api[_-]?key|secret|token)\s*[:=]\s*[\"'][A-Za-z0-9_\-]{24,}[\"']",
    re.IGNORECASE,
)

# The guard's own files mention the patterns they look for; they are exempt
# from the home-path and project-name checks (but not from the others).
GUARD_FILES = {
    "scripts/repo_guard.py",
    "scripts/repo_guard.deny",
    "scripts/repo_guard.allow",
}

# Ignore binaries; they are not text we can judge.
BINARY = re.compile(rb"\x00")


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=REPO, capture_output=True, text=True, check=False
    ).stdout


def staged_files() -> list[str]:
    out = git("diff", "--cached", "--name-only", "--diff-filter=ACMR")
    return [line for line in out.splitlines() if line.strip()]


def tracked_files() -> list[str]:
    return [line for line in git("ls-files").splitlines() if line.strip()]


def deny_patterns() -> list[tuple[str, re.Pattern[str]]]:
    """Project names that must not show up in the repository."""
    patterns: list[tuple[str, re.Pattern[str]]] = []
    for deny_file in (DENY_FILE, LOCAL_DENY_FILE):
        if not deny_file.exists():
            continue
        for raw in deny_file.read_text(encoding="utf-8").splitlines():
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            try:
                patterns.append((line, re.compile(line, re.IGNORECASE)))
            except re.error as exc:
                print(f"repo_guard: bad pattern {line!r}: {exc}", file=sys.stderr)
    return patterns


def allowed_tokens() -> set[str]:
    """Known fake keys that live in the repository's own tests."""
    if not ALLOW_FILE.exists():
        return set()
    return {
        line.strip()
        for line in ALLOW_FILE.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.startswith("#")
    }


def looks_like_key(token: str) -> bool:
    if ".." in token or len(token) < 24:
        return False
    if not SECRET.match(token):
        return False
    # Real keys carry digits; identifiers like "skip_serializing_if" do not.
    return sum(ch.isdigit() for ch in token) >= 4


def check(paths: list[str]) -> list[str]:
    problems: list[str] = []
    denies = deny_patterns()
    allowed = allowed_tokens()
    for rel in paths:
        full = REPO / rel
        if not full.is_file():
            continue
        if FORBIDDEN_PATHS.search(rel):
            problems.append(f"{rel}: must not be tracked (local artefact)")
            continue
        try:
            raw = full.read_bytes()
        except OSError:
            continue
        if BINARY.search(raw[:8192]):
            continue
        text = raw.decode("utf-8", errors="replace")
        if rel not in GUARD_FILES:
            for line_no, line in enumerate(text.splitlines(), 1):
                if HOME_PATH.search(line) or LINUX_HOME.search(line):
                    problems.append(
                        f"{rel}:{line_no}: absolute home path in a tracked file -- "
                        "replace it with a placeholder"
                    )
                    break
            for name, pattern in denies:
                if pattern.search(text):
                    problems.append(
                        f"{rel}: contains the project name {name!r} -- the imc repo "
                        "must not carry anything from a real project"
                    )
                    break
        secret_hit = None
        for token in re.findall(r"[A-Za-z0-9_.\-]{16,}", text):
            if token in allowed:
                continue
            if looks_like_key(token):
                secret_hit = token
                break
        if secret_hit or ASSIGNED_KEY.search(text):
            problems.append(f"{rel}: looks like it contains a real API key")
    return problems


def check_message(path: str) -> list[str]:
    """A commit message can leak a project name just as easily as a file."""
    try:
        text = Path(path).read_text(encoding="utf-8", errors="replace")
    except OSError:
        return []
    problems = []
    for name, pattern in deny_patterns():
        if pattern.search(text):
            problems.append(
                f"commit message: mentions the project name {name!r} -- the imc repo "
                "must not carry anything from a real project"
            )
    for regex in (HOME_PATH, LINUX_HOME):
        match = regex.search(text)
        if match:
            problems.append(
                f"commit message: absolute home path {match.group(0)!r} -- use a placeholder"
            )
            break
    return problems


def report(problems: list[str]) -> int:
    if not problems:
        return 0
    print("repo_guard: refusing the commit\n", file=sys.stderr)
    for p in problems:
        print(f"  x {p}", file=sys.stderr)
    print(
        "\nFix the files above, unstage them with `git restore --staged <file>`, "
        "or add the name to scripts/repo_guard.local.deny if it is a false positive.",
        file=sys.stderr,
    )
    return 1


def main() -> int:
    if "--message" in sys.argv:
        try:
            msg_path = sys.argv[sys.argv.index("--message") + 1]
        except IndexError:
            print("repo_guard: --message needs a file", file=sys.stderr)
            return 2
        problems = check_message(msg_path)
        if problems:
            return report(problems)
        print("repo_guard: commit message clean")
        return 0

    paths = tracked_files() if "--all" in sys.argv else staged_files()
    if not paths:
        print("repo_guard: nothing to check")
        return 0
    problems = check(paths)
    if problems:
        return report(problems)
    print(f"repo_guard: {len(paths)} file(s) clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
