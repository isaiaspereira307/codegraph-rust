#!/usr/bin/env bash
# ABOUTME: Packages a codegraph binary into codegraph-<version>-<target>.tar.gz (.zip on Windows).
# ABOUTME: Usage: package.sh <version> <target> <binary-path> <out-dir>; also writes <asset>.sha256.
set -euo pipefail

if [ "$#" -ne 4 ]; then
  echo "usage: $0 <version> <target> <binary-path> <out-dir>" >&2
  exit 2
fi
version="$1"; target="$2"; binary="$3"; out_dir="$4"
version="${version#v}"

root="$(cd "$(dirname "$0")/../.." && pwd)"
[ -f "$binary" ] || { echo "binary not found: $binary" >&2; exit 1; }
mkdir -p "$out_dir"
out_dir="$(cd "$out_dir" && pwd)"

case "$target" in
  *windows*) exe="codegraph.exe"; ext="zip" ;;
  *) exe="codegraph"; ext="tar.gz" ;;
esac

name="codegraph-${version}-${target}"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/$name"
cp "$binary" "$stage/$name/$exe"
chmod +x "$stage/$name/$exe"
for f in LICENSE-MIT LICENSE-APACHE README.md; do
  cp "$root/$f" "$stage/$name/$f"
done

asset="$name.$ext"
rm -f "$out_dir/$asset"
if [ "$ext" = "zip" ]; then
  if command -v zip >/dev/null 2>&1; then
    (cd "$stage" && zip -qr "$out_dir/$asset" "$name")
  elif command -v 7z >/dev/null 2>&1; then
    (cd "$stage" && 7z a -tzip -bso0 -bsp0 "$out_dir/$asset" "$name")
  else
    echo "neither zip nor 7z found" >&2
    exit 1
  fi
else
  # macOS bsdtar would otherwise add ._* AppleDouble entries
  export COPYFILE_DISABLE=1
  (cd "$stage" && tar -czf "$out_dir/$asset" "$name")
fi

if command -v shasum >/dev/null 2>&1; then
  (cd "$out_dir" && shasum -a 256 "$asset" > "$asset.sha256")
else
  (cd "$out_dir" && sha256sum "$asset" > "$asset.sha256")
fi
echo "$out_dir/$asset"
