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

# pairs: "<os> <arch> <asset> <sha256>" per platform block, taken from on_* nesting.
pairs() {
  awk '/on_macos/{os="macos"} /on_linux/{os="linux"} /on_arm/{a="arm"} /on_intel/{a="intel"}
       /^ *url /{u=$2; gsub(/"/,"",u); sub(/.*\//,"",u)}
       /^ *sha256 /{h=$2; gsub(/"/,"",h); print os, a, u, h}' | sort
}
hash_of() { awk -v a="codegraph-1.2.3-$1.tar.gz" '$2 == a {print $1}' "$2"; }
expected_for() {  # $1 = sums file
  {
    echo "macos arm codegraph-1.2.3-aarch64-apple-darwin.tar.gz $(hash_of aarch64-apple-darwin "$1")"
    echo "macos intel codegraph-1.2.3-x86_64-apple-darwin.tar.gz $(hash_of x86_64-apple-darwin "$1")"
    echo "linux arm codegraph-1.2.3-aarch64-unknown-linux-gnu.tar.gz $(hash_of aarch64-unknown-linux-gnu "$1")"
    echo "linux intel codegraph-1.2.3-x86_64-unknown-linux-gnu.tar.gz $(hash_of x86_64-unknown-linux-gnu "$1")"
  } | sort
}
if [ "$(echo "$out" | pairs)" = "$(expected_for "$tmp/SHA256SUMS")" ]; then
  ok "url/sha256 pairing under correct on_os/on_arch block"
else
  bad "url/sha256 pairing wrong:"; echo "$out" | pairs; echo "--- expected"; expected_for "$tmp/SHA256SUMS"
fi

# negative proof: swapping two hashes in SHA256SUMS must change the pairing, exactly as expected
awk '/aarch64-apple-darwin/{sub(/^[0-9a-f]+/,"'"$(hash_of x86_64-apple-darwin "$tmp/SHA256SUMS")"'")} /x86_64-apple-darwin/{sub(/^[0-9a-f]+/,"'"$(hash_of aarch64-apple-darwin "$tmp/SHA256SUMS")"'")} {print}' "$tmp/SHA256SUMS" > "$tmp/swapped"
sout="$(bash "$render" 1.2.3 "$tmp/swapped")"
if [ "$(echo "$sout" | pairs)" = "$(expected_for "$tmp/swapped")" ] && [ "$(echo "$sout" | pairs)" != "$(echo "$out" | pairs)" ]; then
  ok "swapped hashes are detected by pairing check"
else
  bad "swapped hashes not reflected in pairing"
fi

# binary-mode sums form: "hash *file"
sed 's/  codegraph/ *codegraph/' "$tmp/SHA256SUMS" > "$tmp/star"
bout="$(bash "$render" 1.2.3 "$tmp/star" 2>&1)" && [ "$(echo "$bout" | pairs)" = "$(expected_for "$tmp/SHA256SUMS")" ] \
  && ok "binary-mode '*file' sums accepted" || bad "binary-mode sums rejected"

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
