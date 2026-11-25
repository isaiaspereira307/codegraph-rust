#!/usr/bin/env bash
# ABOUTME: Tests scripts/release/package.sh archive layout, names and checksums.
# ABOUTME: Exit 0 when all assertions hold, 1 otherwise.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
fail=0
ok() { echo "ok: $*"; }
bad() { echo "FAIL: $*"; fail=1; }

printf '#!/bin/sh\necho codegraph 1.2.3\n' > "$tmp/fake"
chmod +x "$tmp/fake"

# unix target
out="$tmp/out1"; mkdir -p "$out"
t=x86_64-unknown-linux-gnu
bash "$root/scripts/release/package.sh" 1.2.3 "$t" "$tmp/fake" "$out" || bad "package.sh failed for $t"
asset="codegraph-1.2.3-$t"
if [ -f "$out/$asset.tar.gz" ]; then
  listing="$(tar -tzf "$out/$asset.tar.gz")"
  for f in codegraph LICENSE-MIT LICENSE-APACHE README.md; do
    echo "$listing" | grep -qx "$asset/$f" && ok "tar has $asset/$f" || bad "tar missing $asset/$f"
  done
  tar -xzOf "$out/$asset.tar.gz" "$asset/codegraph" | cmp -s - "$tmp/fake" && ok "binary content" || bad "binary content differs"
  (cd "$out" && sha256sum -c "$asset.tar.gz.sha256") && ok "sha256 verifies" || bad "sha256 mismatch"
else
  bad "missing $out/$asset.tar.gz"
fi

# windows target
out="$tmp/out2"; mkdir -p "$out"
t=x86_64-pc-windows-msvc
bash "$root/scripts/release/package.sh" 1.2.3 "$t" "$tmp/fake" "$out" || bad "package.sh failed for $t"
asset="codegraph-1.2.3-$t"
if [ -f "$out/$asset.zip" ]; then
  listing="$(unzip -Z1 "$out/$asset.zip")"
  for f in codegraph.exe LICENSE-MIT LICENSE-APACHE README.md; do
    echo "$listing" | grep -qx "$asset/$f" && ok "zip has $asset/$f" || bad "zip missing $asset/$f"
  done
  (cd "$out" && sha256sum -c "$asset.zip.sha256") && ok "sha256 verifies (zip)" || bad "sha256 mismatch (zip)"
else
  bad "missing $out/$asset.zip"
fi

exit "$fail"
