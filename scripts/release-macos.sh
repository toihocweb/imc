#!/usr/bin/env bash
# Build the macOS release assets and publish them as a GitHub release.
#
# Why a script instead of GitHub Actions: the release workflow lives in
# .github/workflows/release.yml, and pushing a workflow file needs a token
# with the `workflow` scope. The classic token on this machine does not have
# it, so the assets are built here, on a Mac, and uploaded with `gh`.
#
# What it does:
#   1. builds the `imc` binary for both macOS architectures,
#   2. signs each one ad-hoc (macOS kills an unsigned Mach-O: "killed",
#      SIGKILL, before it can print anything),
#   3. checks that the binary actually starts,
#   4. creates the GitHub release and uploads the assets.
#
# Usage:
#   ./scripts/release-macos.sh                 # publish the version in Cargo.toml
#   ./scripts/release-macos.sh --dry-run       # build and check, upload nothing
#   ./scripts/release-macos.sh --tag v0.2.0    # publish another tag
#
# Requirements: a Mac with the Rust toolchain, and `gh` authenticated
# (`gh auth login`) with write access to the repository.

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"

DRY_RUN=0
TAG=""
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY_RUN=1; shift ;;
    --tag) TAG="${2:?--tag needs a value}"; shift 2 ;;
    -h|--help) sed -n '2,22p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [ "$(uname -s)" != "Darwin" ]; then
  echo "error: this script builds the macOS assets and has to run on a Mac." >&2
  echo "       on other systems, push .github/workflows/release.yml instead and" >&2
  echo "       tag the release — the workflow builds every platform." >&2
  exit 1
fi

command -v cargo >/dev/null || { echo "error: cargo not found" >&2; exit 1; }
command -v gh >/dev/null || { echo "error: gh not found (brew install gh)" >&2; exit 1; }

# The tag follows the workspace version unless one is given.
if [ -z "$TAG" ]; then
  VERSION="$(awk -F'"' '/^version = /{print $2; exit}' Cargo.toml)"
  [ -n "$VERSION" ] || { echo "error: could not read the version from Cargo.toml" >&2; exit 1; }
  TAG="v${VERSION}"
fi
echo "==> releasing ${TAG}"

# Pull the newest state first: the release has to match what is pushed.
if [ -z "$(git status --porcelain)" ]; then
  echo "==> git pull"
  git pull --ff-only
else
  echo "==> working tree is dirty, skipping git pull"
fi

OUT="dist"
mkdir -p "$OUT"

echo "==> building web dashboard static export"
(cd web-dashboard && npm run build)

for TARGET in aarch64-apple-darwin x86_64-apple-darwin; do
  ARCH="${TARGET%%-*}"
  echo "==> building ${TARGET}"
  rustup target add "$TARGET" >/dev/null
  cargo build --release --bin imc --target "$TARGET"

  BIN="target/${TARGET}/release/imc"
  [ -x "$BIN" ] || { echo "error: ${BIN} was not built" >&2; exit 1; }

  # Ad-hoc signature: macOS refuses to run a Mach-O whose signature does not
  # match the file, and it fails silently (the shell prints "killed").
  echo "==> signing ${TARGET}"
  codesign --force --sign - "$BIN"

  # The check that matters: a binary that cannot start is worse than none.
  echo "==> checking ${TARGET}"
  "$BIN" --version

  cp "$BIN" "${OUT}/imc-darwin-${ARCH}"
done

ls -la "$OUT"

if [ "$DRY_RUN" = "1" ]; then
  echo "==> dry run: nothing uploaded"
  exit 0
fi

echo "==> publishing ${TAG}"
if gh release view "$TAG" >/dev/null 2>&1; then
  gh release upload "$TAG" "${OUT}/imc-darwin-aarch64" "${OUT}/imc-darwin-x86_64" --clobber
else
  gh release create "$TAG" \
    "${OUT}/imc-darwin-aarch64" "${OUT}/imc-darwin-x86_64" \
    --title "imc ${TAG}" \
    --generate-notes
fi

echo
echo "==> done: https://github.com/toihocweb/imc/releases/tag/${TAG}"
echo "    users on macOS can now run: imc update"
