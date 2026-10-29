#!/usr/bin/env bash
#
# install.sh — One-shot installer for imc (IndexMyCode Rust port).
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/toihocweb/imc/master/install.sh | bash
#
# What it does:
#   1. Detects OS + arch (linux-x86_64, linux-aarch64, darwin-x86_64, darwin-aarch64)
#   2. Downloads pre-built binary from GitHub Releases
#   3. Installs to ~/.local/bin (or $IMC_INSTALL_DIR)
#   4. Creates ~/.imc_config.yaml if missing
#
set -euo pipefail

REPO="toihocweb/imc"
BIN_NAME="imc"
INSTALL_DIR="${IMC_INSTALL_DIR:-$HOME/.local/bin}"
CONFIG_FILE="${IMC_CONFIG:-$HOME/.imc_config.yaml}"

info()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn()  { printf '\033[1;33m==>\033[0m %s\n' "$*" >&2; }
error() { printf '\033[1;31m==>\033[0m %s\n' "$*" >&2; exit 1; }

have() { command -v "$1" >/dev/null 2>&1; }

detect_platform() {
    local os arch
    case "$(uname -s)" in
        Linux)  os="linux" ;;
        Darwin) os="darwin" ;;
        *)      error "unsupported OS: $(uname -s)" ;;
    esac

    case "$(uname -m)" in
        x86_64|amd64) arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *)            error "unsupported arch: $(uname -m)" ;;
    esac

    PLATFORM="${os}-${arch}"
}

# Resolve release asset download URL directly from GitHub Releases.
# Supports setting IMC_VERSION (e.g. v0.2.1) or defaults to latest release.
resolve_release_url() {
    local version="${IMC_VERSION:-latest}"
    local url

    if [[ "$version" == "latest" ]]; then
        url="https://github.com/${REPO}/releases/latest/download/${BIN_NAME}-${PLATFORM}"
    else
        url="https://github.com/${REPO}/releases/download/${version}/${BIN_NAME}-${PLATFORM}"
    fi

    # Check direct download URL (does not consume GitHub API rate limits)
    if curl -fsSL --head "$url" >/dev/null 2>&1; then
        echo "$url"
        return 0
    fi

    # Fallback: try GitHub API if latest redirection failed
    local tag
    tag="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null \
        | grep -o '"tag_name": *"[^"]*"' | head -1 | cut -d'"' -f4)" || true

    if [[ -n "${tag:-}" ]]; then
        url="https://github.com/${REPO}/releases/download/${tag}/${BIN_NAME}-${PLATFORM}"
        if curl -fsSL --head "$url" >/dev/null 2>&1; then
            echo "$url"
            return 0
        fi
    fi

    warn "pre-built binary '${BIN_NAME}-${PLATFORM}' not found on GitHub Releases"
    return 1
}

download_binary() {
    local url="$1" dest="$2"
    info "downloading $url"
    if have curl; then
        curl -fsSL "$url" -o "$dest"
    elif have wget; then
        wget -qO "$dest" "$url"
    else
        error "neither curl nor wget is available"
    fi
    chmod +x "$dest"
}

# A downloaded binary carries no signature. macOS kills an unsigned Mach-O
# before it can print anything ("killed", SIGKILL), so sign it ad-hoc.
sign_if_macos() {
    local bin="$1"
    case "$(uname -s)" in
        Darwin)
            if have codesign; then
                codesign --force --sign - "$bin" >/dev/null 2>&1 || true
            fi
            ;;
    esac
}

# Prove the installed binary starts: on macOS a bad signature only shows up
# as the process being killed, so a silent check would leave the user with a
# binary that dies without a message.
verify_binary() {
    local bin="$1"
    if "$bin" --version >/dev/null 2>&1; then
        return 0
    fi
    error "$bin does not run on this machine."
    case "$(uname -s)" in
        Darwin)
            error "on macOS this usually means a missing code signature; try:"
            error "  codesign --force --sign - $bin"
            ;;
    esac
    error "  (rebuild from source: cargo build --release && ./target/release/imc install)"
    return 1
}

install_from_release() {
    local url tmp dest
    url="$(resolve_release_url)" || return 1
    tmp="$(mktemp)"
    download_binary "$url" "$tmp"

    mkdir -p "$INSTALL_DIR"
    dest="$INSTALL_DIR/$BIN_NAME"
    mv "$tmp" "$dest"
    chmod +x "$dest"
    rm -f "$tmp"
    sign_if_macos "$dest"
    info "installed $BIN_NAME to $dest"
    verify_binary "$dest" || return 1
}

build_from_source() {
    info "building imc from source (this may take a few minutes)"
    if ! have cargo; then
        warn "cargo not found; installing rust via rustup"
        if ! curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y; then
            error "failed to install rust"
        fi
        # shellcheck source=/dev/null
        . "$HOME/.cargo/env"
    fi

    local tmp
    tmp="$(mktemp -d)"
    git clone --depth 1 "https://github.com/${REPO}.git" "$tmp"
    (cd "$tmp" && cargo build --release)
    mkdir -p "$INSTALL_DIR"
    cp "$tmp/target/release/$BIN_NAME" "$INSTALL_DIR/$BIN_NAME"
    chmod +x "$INSTALL_DIR/$BIN_NAME"
    rm -rf "$tmp"
    sign_if_macos "$INSTALL_DIR/$BIN_NAME"
    info "installed $BIN_NAME to $INSTALL_DIR/$BIN_NAME"
    verify_binary "$INSTALL_DIR/$BIN_NAME" || return 1
}

ensure_config() {
    if [[ ! -f "$CONFIG_FILE" ]]; then
        mkdir -p "$(dirname "$CONFIG_FILE")"
        cat > "$CONFIG_FILE" <<'EOF'
# imc configuration — edit with `imc config set ...`
llm:
  base_url: http://localhost:6655/litellm/v1
  api_key: ""
  model: claude-sonnet-4-6
embedding:
  model: text-embedding-3-small
EOF
        chmod 600 "$CONFIG_FILE"
        info "wrote starter config to $CONFIG_FILE"
    else
        info "config already exists at $CONFIG_FILE (left untouched)"
    fi
}

check_path() {
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) return 0 ;;
    esac
    warn "$INSTALL_DIR is not on your PATH. Add one of the following to your shell rc file:"
    echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
}

main() {
    detect_platform
    info "platform: $PLATFORM"

    if ! install_from_release; then
        if [[ "${IMC_BUILD_FROM_SOURCE:-0}" == "1" ]]; then
            info "falling back to building from source..."
            build_from_source
        else
            error "Failed to install pre-built release binary for $PLATFORM from GitHub Releases ($REPO). If you wish to compile from source instead, set IMC_BUILD_FROM_SOURCE=1"
        fi
    fi

    ensure_config
    check_path

    info "done. Try: $BIN_NAME --help"
}

main "$@"
