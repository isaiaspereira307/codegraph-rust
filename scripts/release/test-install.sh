#!/usr/bin/env bash
# ABOUTME: Tests install.sh against a fake file:// release built with package.sh.
# ABOUTME: Covers install into a spaced dir, checksum mismatch, unsupported OS, arch mapping, --help.
set -uo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
installer="$root/install.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
fail=0
ok()  { echo "PASS: $1"; }
bad() { echo "FAIL: $1"; fail=1; }

version="1.2.3"
fake="$tmp/fake-codegraph"
printf '#!/bin/sh\necho "codegraph 1.2.3"\n' > "$fake"
chmod +x "$fake"

# make_release <dir> <target...>: asset + .sha256 per target, combined SHA256SUMS.
make_release() {
  local dir="$1"; shift
  local rel="$dir/download/v$version" t
  mkdir -p "$rel"
  for t in "$@"; do
    bash "$root/scripts/release/package.sh" "$version" "$t" "$fake" "$rel" >/dev/null || return 1
  done
  cat "$rel"/*.sha256 | sort -k2 > "$rel/SHA256SUMS"
}

make_release "$tmp/rel" x86_64-unknown-linux-gnu aarch64-apple-darwin || { echo "release build failed"; exit 1; }
base="file://$tmp/rel"

run_install() { # env passed via caller; args: dest
  CODEGRAPH_RELEASE_BASE_URL="$base" CODEGRAPH_VERSION="$version" \
    CODEGRAPH_INSTALL_DIR="$1" sh "$installer" 2>&1
}

# (a) install into a dir with spaces
dest="$tmp/my bin"
out="$(CODEGRAPH_TEST_UNAME_S=Linux CODEGRAPH_TEST_UNAME_M=x86_64 run_install "$dest")"; rc=$?
if [ $rc -eq 0 ] && [ "$("$dest/codegraph" --version 2>&1)" = "codegraph 1.2.3" ]; then
  ok "installs into dir with spaces and binary runs"
else
  bad "install into spaced dir (rc=$rc): $out"
fi

# (b) adulterated checksum
make_release "$tmp/bad" x86_64-unknown-linux-gnu
sed -i.bak 's/^./0/' "$tmp/bad/download/v$version/SHA256SUMS"
dest="$tmp/bad dest"
out="$(CODEGRAPH_TEST_UNAME_S=Linux CODEGRAPH_TEST_UNAME_M=x86_64 \
  CODEGRAPH_RELEASE_BASE_URL="file://$tmp/bad" CODEGRAPH_VERSION="$version" \
  CODEGRAPH_INSTALL_DIR="$dest" sh "$installer" 2>&1)"; rc=$?
if [ $rc -eq 1 ] && { [ ! -e "$dest" ] || [ -z "$(ls -A "$dest")" ]; }; then
  ok "checksum mismatch exits 1 and leaves destination empty"
else
  bad "checksum mismatch (rc=$rc): $out"
fi

# (c) unsupported OS: exit 1, message lists targets, nothing downloaded
stubs="$tmp/stubs"; mkdir -p "$stubs"
printf '#!/bin/sh\necho "$@" >> "%s/calls.log"\nexit 1\n' "$tmp" > "$stubs/curl"
cp "$stubs/curl" "$stubs/wget"; chmod +x "$stubs/curl" "$stubs/wget"
out="$(PATH="$stubs:$PATH" CODEGRAPH_TEST_UNAME_S=FreeBSD CODEGRAPH_TEST_UNAME_M=x86_64 \
  CODEGRAPH_RELEASE_BASE_URL="$base" CODEGRAPH_VERSION="$version" \
  CODEGRAPH_INSTALL_DIR="$tmp/fbsd" sh "$installer" 2>&1)"; rc=$?
if [ $rc -eq 1 ] && printf '%s' "$out" | grep -q 'x86_64-unknown-linux-gnu' \
   && [ ! -e "$tmp/calls.log" ] && [ ! -e "$tmp/fbsd" ]; then
  ok "FreeBSD rejected with supported targets, nothing downloaded"
else
  bad "FreeBSD (rc=$rc): $out"
fi

# (d) Darwin arm64 maps to aarch64-apple-darwin
dest="$tmp/darwin"
out="$(CODEGRAPH_TEST_UNAME_S=Darwin CODEGRAPH_TEST_UNAME_M=arm64 run_install "$dest")"; rc=$?
if [ $rc -eq 0 ] && printf '%s' "$out" | grep -q 'aarch64-apple-darwin' && [ -x "$dest/codegraph" ]; then
  ok "Darwin arm64 maps to aarch64-apple-darwin"
else
  bad "Darwin arm64 (rc=$rc): $out"
fi

# (e) --help
out="$(sh "$installer" --help 2>&1)"; rc=$?
if [ $rc -eq 0 ] && printf '%s' "$out" | grep -q 'CODEGRAPH_INSTALL_DIR'; then
  ok "--help exits 0"
else
  bad "--help (rc=$rc): $out"
fi

exit $fail
