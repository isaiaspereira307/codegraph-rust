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
if [ $rc -eq 1 ] && printf '%s' "$out" | grep -qi 'checksum' && { [ ! -e "$dest" ] || [ -z "$(ls -A "$dest")" ]; }; then
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

# (f) invalid versions rejected before any download
: > "$tmp/calls.log"; rm -f "$tmp/calls.log"
for v in '../x' '1.2.3/evil' '1.2?x'; do
  out="$(PATH="$stubs:$PATH" CODEGRAPH_TEST_UNAME_S=Linux CODEGRAPH_TEST_UNAME_M=x86_64 \
    CODEGRAPH_RELEASE_BASE_URL="$base" CODEGRAPH_VERSION="$v" \
    CODEGRAPH_INSTALL_DIR="$tmp/inv" sh "$installer" 2>&1)"; rc=$?
  if [ $rc -eq 1 ] && printf '%s' "$out" | grep -qi 'invalid version' && [ ! -e "$tmp/calls.log" ]; then
    ok "invalid version '$v' rejected without download"
  else
    bad "invalid version '$v' (rc=$rc): $out"
  fi
done

# (g) SHA256SUMS without the asset's entry
make_release "$tmp/miss" x86_64-unknown-linux-gnu
echo "0000  other-file.tar.gz" > "$tmp/miss/download/v$version/SHA256SUMS"
out="$(CODEGRAPH_TEST_UNAME_S=Linux CODEGRAPH_TEST_UNAME_M=x86_64 \
  CODEGRAPH_RELEASE_BASE_URL="file://$tmp/miss" CODEGRAPH_VERSION="$version" \
  CODEGRAPH_INSTALL_DIR="$tmp/miss dest" sh "$installer" 2>&1)"; rc=$?
if [ $rc -eq 1 ] && [ ! -e "$tmp/miss dest" ]; then ok "missing SHA256SUMS entry exits 1"; else bad "missing entry (rc=$rc): $out"; fi

# (h) overwrite existing binary: works, no temp files left
dest="$tmp/over"; mkdir -p "$dest"
printf '#!/bin/sh\necho old\n' > "$dest/codegraph"; chmod +x "$dest/codegraph"
out="$(CODEGRAPH_TEST_UNAME_S=Linux CODEGRAPH_TEST_UNAME_M=x86_64 run_install "$dest")"; rc=$?
if [ $rc -eq 0 ] && [ "$("$dest/codegraph" --version)" = "codegraph 1.2.3" ] \
   && [ "$(ls -A "$dest")" = "codegraph" ]; then
  ok "overwrite leaves new binary and no temp files"
else
  bad "overwrite (rc=$rc): $(ls -A "$dest"): $out"
fi

# (i) mismatch keeps an existing old binary byte-identical
dest="$tmp/keep"; mkdir -p "$dest"
printf '#!/bin/sh\necho old\n' > "$dest/codegraph"; chmod +x "$dest/codegraph"
cp "$dest/codegraph" "$tmp/old.copy"
out="$(CODEGRAPH_TEST_UNAME_S=Linux CODEGRAPH_TEST_UNAME_M=x86_64 \
  CODEGRAPH_RELEASE_BASE_URL="file://$tmp/bad" CODEGRAPH_VERSION="$version" \
  CODEGRAPH_INSTALL_DIR="$dest" sh "$installer" 2>&1)"; rc=$?
if [ $rc -eq 1 ] && cmp -s "$dest/codegraph" "$tmp/old.copy" && [ "$(ls -A "$dest")" = "codegraph" ]; then
  ok "mismatch keeps existing binary intact"
else
  bad "keep old (rc=$rc): $out"
fi

exit $fail
