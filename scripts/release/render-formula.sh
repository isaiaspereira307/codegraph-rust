#!/usr/bin/env bash
# ABOUTME: Renders the Homebrew formula from packaging/homebrew/codegraph.rb.tmpl to stdout.
# ABOUTME: Usage: render-formula.sh <version> <SHA256SUMS-path>; exit 1 on bad version or missing hash.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: render-formula.sh <version> <SHA256SUMS-path>" >&2
  exit 2
fi
version="$1"
sums="$2"
root="$(cd "$(dirname "$0")/../.." && pwd)"
tmpl="$root/packaging/homebrew/codegraph.rb.tmpl"

case "$version" in
  ''|*[!0-9A-Za-z._-]*) echo "invalid version: '$version'" >&2; exit 1 ;;
esac
[ -f "$sums" ] || { echo "SHA256SUMS not found: $sums" >&2; exit 1; }
[ -f "$tmpl" ] || { echo "template not found: $tmpl" >&2; exit 1; }

targets="x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-apple-darwin aarch64-apple-darwin"
sed_args=(-e "s/@VERSION@/$version/g")
for t in $targets; do
  asset="codegraph-$version-$t.tar.gz"
  hash="$(awk -v a="$asset" '$2 == a { print $1; exit }' "$sums")"
  if ! printf '%s' "$hash" | grep -Eq '^[0-9a-f]{64}$'; then
    echo "missing or invalid sha256 for $asset in $sums" >&2
    exit 1
  fi
  key="$(printf '%s' "$t" | tr 'a-z-' 'A-Z_')"
  sed_args+=(-e "s/@SHA_${key}@/$hash/g")
done

sed "${sed_args[@]}" "$tmpl"
