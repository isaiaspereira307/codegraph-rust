#!/usr/bin/env bash
# ABOUTME: Tests scripts/release/render-formula.sh against a fake SHA256SUMS for the 4 unix targets.
# ABOUTME: Checks URLs, hashes, leftover placeholders, missing-hash failure, bad version, ruby -c.
set -uo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
render="$root/scripts/release/render-formula.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
fail=0
ok()  { echo "PASS: $1"; }
bad() { echo "FAIL: $1"; fail=1; }

targets="x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-apple-darwin aarch64-apple-darwin"
base="https://github.com/isaiaspereira307/codegraph-rust/releases/download/v1.2.3"
n=0
: > "$tmp/SHA256SUMS"
for t in $targets; do
  n=$((n + 1))
  hash="$(printf '%064d' "$n" | tr '0' 'a' | sed "s/^./$n/")"
  echo "$hash  codegraph-1.2.3-$t.tar.gz" >> "$tmp/SHA256SUMS"
done

out="$(bash "$render" 1.2.3 "$tmp/SHA256SUMS" 2>"$tmp/err")" || { bad "render failed: $(cat "$tmp/err")"; out=""; }

while read -r hash name; do
  t="${name#codegraph-1.2.3-}"; t="${t%.tar.gz}"
  if echo "$out" | grep -qF "url \"$base/$name\""; then ok "url for $t"; else bad "url for $t"; fi
  if echo "$out" | grep -qF "sha256 \"$hash\""; then ok "hash for $t"; else bad "hash for $t"; fi
done < "$tmp/SHA256SUMS"

echo "$out" | grep -q '@[A-Za-z0-9_]*@' && bad "placeholder left over" || ok "no placeholders left"
echo "$out" | grep -qF 'version "1.2.3"' && ok "version line" || bad "version line"
echo "$out" | grep -qF 'bin.install "codegraph"' && ok "bin.install" || bad "bin.install"

# missing hash -> exit 1
grep -v aarch64-apple-darwin "$tmp/SHA256SUMS" > "$tmp/missing"
bash "$render" 1.2.3 "$tmp/missing" >/dev/null 2>"$tmp/err"; rc=$?
if [ "$rc" -eq 1 ] && grep -q 'aarch64-apple-darwin' "$tmp/err"; then ok "missing hash exits 1 with message"; else bad "missing hash rc=$rc"; fi

# bad version -> exit 1
bash "$render" '1.2/3' "$tmp/SHA256SUMS" >/dev/null 2>&1; rc=$?
[ "$rc" -eq 1 ] && ok "bad version exits 1" || bad "bad version rc=$rc"

if command -v ruby >/dev/null 2>&1; then
  echo "$out" > "$tmp/codegraph.rb"
  ruby -c "$tmp/codegraph.rb" >/dev/null && ok "ruby -c" || bad "ruby -c"
else
  echo "NOTICE: ruby not found, skipping ruby -c"
fi

exit "$fail"
