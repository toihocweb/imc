#!/usr/bin/env bash
# Install the repository guard as git hooks.
#
#   pre-commit  → checks the staged files
#   commit-msg  → checks the commit message (git passes the message file here)
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$root/.git/hooks"

cat > "$root/.git/hooks/pre-commit" <<EOF
#!/usr/bin/env bash
# Refuse commits that carry anything from a real project.
"$root/scripts/repo_guard.py" || exit 1
EOF

cat > "$root/.git/hooks/commit-msg" <<EOF
#!/usr/bin/env bash
# Refuse commit messages that name a real project.
"$root/scripts/repo_guard.py" --message "\${1:-}" || exit 1
EOF

chmod +x "$root/.git/hooks/pre-commit" "$root/.git/hooks/commit-msg"
echo "installed pre-commit + commit-msg hooks"
