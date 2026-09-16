#!/usr/bin/env bash
#
# hfd installer — macOS, Ubuntu/Debian, Arch Linux (and most other distros).
#
#   curl -fsSL https://raw.githubusercontent.com/iamhsouna/hfd/master/install.sh | bash
#
# Options:
#   --from-source     Build from source instead of using a prebuilt binary
#   --version TAG     Install a specific release tag (default: latest)
#   --bin-dir DIR     Install into DIR (default: ~/.local/bin)
#   --no-modify-path  Do not touch shell startup files
#   --no-deps         Skip installing system dependencies
#   --help            Show this help

set -euo pipefail

REPO="${HFD_REPO:-iamhsouna/hfd}"
BIN_NAME="hfd"
VERSION="${HFD_VERSION:-latest}"
FROM_SOURCE=0
INSTALL_DEPS=1
MODIFY_PATH=1
BIN_DIR=""

# ---------------------------------------------------------------- output ----

if [ -t 1 ]; then
  C_RESET="$(printf '\033[0m')"; C_BOLD="$(printf '\033[1m')"
  C_RED="$(printf '\033[31m')"; C_GREEN="$(printf '\033[32m')"
  C_YELLOW="$(printf '\033[33m')"; C_CYAN="$(printf '\033[36m')"
else
  C_RESET=""; C_BOLD=""; C_RED=""; C_GREEN=""; C_YELLOW=""; C_CYAN=""
fi

info() { printf '%s\n' "${C_CYAN}==>${C_RESET} $*"; }
ok()   { printf '%s\n' "${C_GREEN}✓${C_RESET} $*"; }
warn() { printf '%s\n' "${C_YELLOW}!${C_RESET} $*" >&2; }
err()  { printf '%s\n' "${C_RED}error:${C_RESET} $*" >&2; }
die()  { err "$*"; exit 1; }

usage() {
  sed -n '2,14p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//' || true
}

# ---------------------------------------------------------------- args ------

while [ $# -gt 0 ]; do
  case "$1" in
    --from-source) FROM_SOURCE=1; shift ;;
    --version) VERSION="${2:-}"; shift 2 ;;
    --version=*) VERSION="${1#*=}"; shift ;;
    --bin-dir) BIN_DIR="${2:-}"; shift 2 ;;
    --bin-dir=*) BIN_DIR="${1#*=}"; shift ;;
    --no-modify-path) MODIFY_PATH=0; shift ;;
    --no-deps) INSTALL_DEPS=0; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1 (try --help)" ;;
  esac
done

# ---------------------------------------------------------------- detect ----

OS="$(uname -s)"
ARCH_RAW="$(uname -m)"

case "$OS" in
  Darwin) PLATFORM="macos" ;;
  Linux)  PLATFORM="linux" ;;
  *) die "unsupported operating system: $OS (macOS and Linux are supported)" ;;
esac

case "$ARCH_RAW" in
  arm64|aarch64) ARCH="arm64" ;;
  x86_64|amd64)  ARCH="x86_64" ;;
  *) die "unsupported architecture: $ARCH_RAW" ;;
esac

case "$PLATFORM-$ARCH" in
  macos-arm64)   TARGET="aarch64-apple-darwin" ;;
  macos-x86_64)  TARGET="x86_64-apple-darwin" ;;
  linux-x86_64)  TARGET="x86_64-unknown-linux-gnu" ;;
  linux-arm64)   TARGET="aarch64-unknown-linux-gnu" ;;
  *) die "no prebuilt target for $PLATFORM-$ARCH" ;;
esac

DISTRO_ID=""
DISTRO_LIKE=""
if [ -r /etc/os-release ]; then
  # shellcheck disable=SC1091
  . /etc/os-release
  DISTRO_ID="${ID:-}"
  DISTRO_LIKE="${ID_LIKE:-}"
fi

if [ -z "$BIN_DIR" ]; then
  BIN_DIR="$HOME/.local/bin"
fi

SUDO=""
if [ "$(id -u)" -ne 0 ]; then
  if command -v sudo >/dev/null 2>&1; then
    SUDO="sudo"
  fi
fi

# Directory of this script when it is a real file (i.e. not piped to bash).
SCRIPT_DIR=""
if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fi

have() { command -v "$1" >/dev/null 2>&1; }

# ------------------------------------------------------------ dependencies --

install_deps() {
  if [ "$INSTALL_DEPS" -eq 0 ]; then
    warn "skipping system dependencies (--no-deps)"
    return 0
  fi

  info "installing system dependencies for $PLATFORM${DISTRO_ID:+ ($DISTRO_ID)}"

  case "$PLATFORM" in
    macos)
      if ! have curl; then
        die "curl is required; install it with: brew install curl"
      fi
      if have brew; then
        brew list git >/dev/null 2>&1 || brew install git >/dev/null 2>&1 || true
      fi
      if [ "$FROM_SOURCE" -eq 1 ] && ! xcode-select -p >/dev/null 2>&1; then
        warn "Xcode Command Line Tools are required to build from source."
        warn "Run: xcode-select --install   then re-run this installer."
        die "missing command line tools"
      fi
      ;;
    linux)
      case "$DISTRO_ID $DISTRO_LIKE" in
        *arch*)
          $SUDO pacman -Sy --needed --noconfirm base-devel curl git ca-certificates >/dev/null
          ;;
        *ubuntu*|*debian*|*mint*|*pop*)
          $SUDO apt-get update -y >/dev/null
          $SUDO DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
            curl git ca-certificates \
            build-essential pkg-config >/dev/null
          ;;
        *fedora*|*rhel*|*centos*)
          $SUDO dnf install -y curl git ca-certificates \
            gcc gcc-c++ make pkgconf-pkg-config >/dev/null
          ;;
        *suse*)
          $SUDO zypper --non-interactive install curl git ca-certificates \
            gcc gcc-c++ make pkg-config >/dev/null
          ;;
        *alpine*)
          $SUDO apk add --no-cache curl git ca-certificates build-base >/dev/null
          ;;
        *)
          if have apt-get; then
            $SUDO apt-get update -y >/dev/null
            $SUDO DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
              curl git ca-certificates build-essential pkg-config >/dev/null
          elif have pacman; then
            $SUDO pacman -Sy --needed --noconfirm base-devel curl git ca-certificates >/dev/null
          else
            warn "unknown distribution — please ensure curl, git and a C toolchain are installed"
          fi
          ;;
      esac
      ;;
  esac
  ok "dependencies ready"
}

ensure_rust() {
  if have cargo; then
    ok "found cargo ($(cargo --version))"
    return 0
  fi
  info "installing Rust via rustup"
  have curl || die "curl is required to install Rust"
  curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain stable >/dev/null
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
  ok "installed $(cargo --version)"
}

# --------------------------------------------------------------- helpers ----

latest_tag() {
  curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' \
    | head -n 1
}

resolve_tag() {
  if [ "$VERSION" = "latest" ]; then
    latest_tag
  else
    case "$VERSION" in
      v*) printf '%s' "$VERSION" ;;
      *) printf 'v%s' "$VERSION" ;;
    esac
  fi
}

download() {
  local url="$1" dest="$2"
  curl -fL --retry 3 --connect-timeout 20 --proto '=https' -o "$dest" "$url"
}

install_to() {
  local src="$1" dir="$2"
  local target="$dir/$BIN_NAME"
  if [ -w "$dir" ] || [ ! -e "$dir" ]; then
    mkdir -p "$dir"
    if [ -e "$target" ] && [ ! -w "$target" ]; then
      $SUDO install -m 0755 "$src" "$target"
    else
      install -m 0755 "$src" "$target"
    fi
  else
    $SUDO install -m 0755 "$src" "$target"
  fi
  printf '%s' "$target"
}

# ---------------------------------------------------------------- install ----

install_binary() {
  local tag url tmp expected actual
  tag="$(resolve_tag)"
  if [ -z "$tag" ]; then
    return 1
  fi

  url="https://github.com/$REPO/releases/download/$tag/$BIN_NAME-$TARGET"
  tmp="$(mktemp)"
  info "downloading $BIN_NAME $tag ($TARGET)"
  if ! download "$url" "$tmp"; then
    rm -f "$tmp"
    return 1
  fi

  if download "$url.sha256" "$tmp.sha256" 2>/dev/null; then
    expected="$(cut -d' ' -f1 < "$tmp.sha256")"
    actual="$(if have sha256sum; then sha256sum "$tmp" | cut -d' ' -f1; else shasum -a 256 "$tmp" | cut -d' ' -f1; fi)"
    rm -f "$tmp.sha256"
    if [ -n "$expected" ] && [ "$expected" != "$actual" ]; then
      rm -f "$tmp"
      die "checksum mismatch (expected $expected, got $actual)"
    fi
    ok "checksum verified"
  fi

  chmod 0755 "$tmp"
  local installed
  installed="$(install_to "$tmp" "$BIN_DIR")"
  rm -f "$tmp"
  ok "installed $installed"
}

install_source() {
  ensure_rust
  have git || die "git is required to build from source"

  local tag src local_build=0
  if [ -n "$SCRIPT_DIR" ] && [ -f "$SCRIPT_DIR/Cargo.toml" ]; then
    src="$SCRIPT_DIR"
    local_build=1
    info "building hfd from the local checkout ($src)"
  else
    tag="$(resolve_tag)"
    src="$(mktemp -d)"
    info "cloning $REPO${tag:+ ($tag)}"
    if [ -n "$tag" ]; then
      git clone --depth 1 --branch "$tag" "https://github.com/$REPO.git" "$src" >/dev/null 2>&1 \
        || git clone --depth 1 "https://github.com/$REPO.git" "$src" >/dev/null 2>&1 \
        || die "failed to clone $REPO"
    else
      git clone --depth 1 "https://github.com/$REPO.git" "$src" >/dev/null 2>&1 \
        || die "failed to clone $REPO"
    fi
  fi

  info "building hfd from source (this can take a few minutes)"
  ( cd "$src" && cargo build --release --locked )

  local installed
  installed="$(install_to "$src/target/release/$BIN_NAME" "$BIN_DIR")"
  if [ "$local_build" -eq 0 ]; then
    rm -rf "$src"
  fi
  ok "installed $installed"
}

# ------------------------------------------------------------------ PATH ----

update_path() {
  [ "$MODIFY_PATH" -eq 1 ] || return 0
  case ":$PATH:" in
    *":$BIN_DIR:"*) return 0 ;;
  esac
  local line="export PATH=\"$BIN_DIR:\$PATH\""
  local rc
  for rc in "$HOME/.zshrc" "$HOME/.bashrc" "$HOME/.profile"; do
    if [ -e "$rc" ] && ! grep -Fq "$BIN_DIR" "$rc" 2>/dev/null; then
      printf '\n# Added by the hfd installer\n%s\n' "$line" >> "$rc"
      ok "added $BIN_DIR to PATH in $rc"
    fi
  done
  warn "restart your shell (or run: export PATH=\"$BIN_DIR:\$PATH\")"
}

# ------------------------------------------------------------------ main ----

printf '%s\n' "${C_BOLD}hfd installer${C_RESET}"
printf '  platform : %s %s (%s)\n' "$PLATFORM" "$ARCH" "$TARGET"
printf '  repo     : %s\n' "$REPO"
printf '  bin dir  : %s\n\n' "$BIN_DIR"

install_deps

if [ "$FROM_SOURCE" -eq 1 ]; then
  install_source
else
  if ! install_binary; then
    warn "no prebuilt binary available — falling back to a source build"
    FROM_SOURCE=1
    install_source
  fi
fi

update_path

printf '\n'
ok "hfd installed successfully"
if have "$BIN_NAME"; then
  "$BIN_NAME" --version || true
else
  printf '  Run: %s/%s --version\n' "$BIN_DIR" "$BIN_NAME"
fi
printf '  Update later with: %s\n' "hfd update"
