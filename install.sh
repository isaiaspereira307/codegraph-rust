#!/bin/sh
# ABOUTME: Installs a prebuilt codegraph binary from GitHub Releases (curl | sh friendly).
# ABOUTME: Verifies the asset against SHA256SUMS before extracting; see --help for variables.
set -eu

DEFAULT_BASE="https://github.com/isaiaspereira307/codegraph-rust/releases"
SUPPORTED="x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu, x86_64-apple-darwin, aarch64-apple-darwin"

usage() {
  cat <<EOH
Install the prebuilt codegraph binary.

Usage: install.sh [-h|--help]

Environment variables:
  CODEGRAPH_VERSION           Version to install, e.g. 1.2.3 or v1.2.3 (default: latest)
  CODEGRAPH_INSTALL_DIR       Install directory (default: \$HOME/.local/bin)
  CODEGRAPH_RELEASE_BASE_URL  Releases base URL (default: $DEFAULT_BASE);
                              file://<dir> uses <dir>/download/v<version>/ (needs CODEGRAPH_VERSION)

Supported targets: $SUPPORTED
EOH
}

err() { echo "error: $*" >&2; }

case "${1:-}" in
  -h|--help) usage; exit 0 ;;
  "") ;;
  *) err "unknown argument: $1"; usage >&2; exit 2 ;;
esac

# --- platform detection (before any network access) ---
# CODEGRAPH_TEST_UNAME_* are test hooks; they override uname for the test suite only.
os="${CODEGRAPH_TEST_UNAME_S:-$(uname -s)}"
arch="${CODEGRAPH_TEST_UNAME_M:-$(uname -m)}"
case "$os" in
  MINGW*|MSYS*|CYGWIN*|Windows*)
    err "Windows is not supported by this script."
    echo "Download the .zip for Windows from the Releases page: $DEFAULT_BASE" >&2
    echo "Supported targets: $SUPPORTED" >&2
    exit 1 ;;
esac
case "$os/$arch" in
  Linux/x86_64|Linux/amd64)    target="x86_64-unknown-linux-gnu" ;;
  Linux/aarch64|Linux/arm64)   target="aarch64-unknown-linux-gnu" ;;
  Darwin/x86_64)               target="x86_64-apple-darwin" ;;
  Darwin/arm64|Darwin/aarch64) target="aarch64-apple-darwin" ;;
  *)
    err "unsupported platform: $os $arch"
    echo "Supported targets: $SUPPORTED" >&2
    exit 1 ;;
esac

base="${CODEGRAPH_RELEASE_BASE_URL:-$DEFAULT_BASE}"
base="${base%/}"
if [ -n "${CODEGRAPH_INSTALL_DIR:-}" ]; then
  install_dir="$CODEGRAPH_INSTALL_DIR"
elif [ -n "${HOME:-}" ]; then
  install_dir="$HOME/.local/bin"
else
  err "HOME is not set; set CODEGRAPH_INSTALL_DIR"; exit 1
fi
version="${CODEGRAPH_VERSION:-latest}"

# --- downloader ---
if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -qO "$2" "$1"; }
else
  err "neither curl nor wget found"; exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | awk '{print $1}'; }
else
  err "neither sha256sum nor shasum found"; exit 1
fi

# --- resolve version ---
if [ "$version" = "latest" ]; then
  case "$base" in
    file://*)
      err "CODEGRAPH_VERSION=latest cannot be resolved for a file:// base URL; set CODEGRAPH_VERSION explicitly (e.g. 1.2.3)"
      exit 1 ;;
  esac
  if command -v curl >/dev/null 2>&1; then
    final="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$base/latest")" || { err "could not resolve latest release"; exit 1; }
  else
    final="$(wget -q --spider -S "$base/latest" 2>&1 | awk 'tolower($1)=="location:"{u=$2} END{print u}' | tr -d '\r')"
  fi
  version="${final##*/}"
  case "$version" in v[0-9]*) ;; *) err "could not determine latest version from: $final"; exit 1 ;; esac
fi
version="${version#v}"
case "$version" in
  ''|*[!0-9A-Za-z._-]*) err "invalid version: '$version'"; exit 1 ;;
esac

asset="codegraph-${version}-${target}.tar.gz"
url_dir="$base/download/v$version"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

echo "Installing codegraph $version for $target"
echo "Downloading $url_dir/$asset"
fetch "$url_dir/$asset" "$tmp/$asset" || { err "download failed: $url_dir/$asset"; exit 1; }
fetch "$url_dir/SHA256SUMS" "$tmp/SHA256SUMS" || { err "download failed: $url_dir/SHA256SUMS"; exit 1; }

# --- verify before extracting ---
expected="$(awk -v f="$asset" '$2==f || $2=="*"f {print $1; exit}' "$tmp/SHA256SUMS")"
[ -n "$expected" ] || { err "$asset not listed in SHA256SUMS"; exit 1; }
actual="$(sha256 "$tmp/$asset")"
if [ "$expected" != "$actual" ]; then
  err "checksum mismatch for $asset (expected $expected, got $actual); nothing was installed"
  exit 1
fi

mkdir -p "$tmp/extract"
tar -xzf "$tmp/$asset" -C "$tmp/extract" "codegraph-${version}-${target}/codegraph"
bin="$tmp/extract/codegraph-${version}-${target}/codegraph"
[ -f "$bin" ] || { err "codegraph binary not found in archive"; exit 1; }
chmod +x "$bin"

mkdir -p "$install_dir"
staged="$install_dir/.codegraph.$$"
trap 'rm -rf "$tmp" "$staged"' EXIT
cp "$bin" "$staged" || { err "could not write to $install_dir"; exit 1; }
chmod +x "$staged"
mv -f "$staged" "$install_dir/codegraph"
echo "Installed: $install_dir/codegraph"

case ":${PATH:-}:" in
  *":$install_dir:"*) ;;
  *) echo "warning: $install_dir is not in your PATH; add it, e.g. export PATH=\"$install_dir:\$PATH\"" >&2 ;;
esac

cat <<EOM

Next steps (from the project you want to index):
  codegraph init .        # choose Claude Code / Codex hooks, add instructions, build the graph
  codegraph doctor .      # check that everything is wired up
EOM
