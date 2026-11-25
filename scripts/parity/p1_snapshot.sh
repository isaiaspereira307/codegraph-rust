#!/usr/bin/env bash
# ABOUTME: Frozen P1 parity snapshot: corpus, probes, inheritance, 3 change scenarios.
# ABOUTME: Usage: scripts/parity/p1_snapshot.sh <out-dir> (never writes under docs/).
set -euo pipefail

if [ $# -ne 1 ]; then
    echo "usage: $0 <out-dir>" >&2
    exit 2
fi

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
OUT=$(mkdir -p "$1" && cd "$1" && pwd)
CG="$ROOT/target/debug/codegraph"
CRG_ORIG=$(cd "$ROOT/../code-review-graph" && pwd)
CRG_REPO="$OUT/work/crg-repo"
CRG_PIN=bbdda4b
GIT=(git -c user.name=parity -c user.email=parity@localhost
     -c commit.gpgsign=false -c core.hooksPath=/dev/null)
CRG=(uv run --frozen --extra communities --project "$CRG_REPO" code-review-graph)
CRG_PY=(uv run --frozen --extra communities --project "$CRG_REPO" python)

mkdir -p "$OUT"/{work,corpus,probes,inheritance,changes}

now() { python3 -c 'import time; print(time.monotonic())'; }
elapsed() { python3 -c "print(round($2 - $1, 2))"; }

echo "[1/7] build codegraph and pin CRG at $CRG_PIN"
(cd "$ROOT" && cargo build -q -p codegraph-mcp-server --bin codegraph)
git clone -q --shared "$CRG_ORIG" "$CRG_REPO"
"${GIT[@]}" -C "$CRG_REPO" checkout -q "$CRG_PIN"

echo "[2/7] corpus"
python3 scripts/parity/parity.py corpus --work "$OUT/work/corpus" \
    --crg-repo "$CRG_REPO" --codegraph "$CG" --md "$OUT/corpus.md"

echo "[3/7] probes"
for probe in python python-imports rust-imports inheritance typescript-js go-package java-cross-file java-framework; do
    python3 scripts/parity/parity.py run "$ROOT/scripts/parity/probes/$probe" \
        --work "$OUT/work/$probe" --crg-repo "$CRG_REPO" --codegraph "$CG" \
        --json "$OUT/probes/$probe.json" --md "$OUT/probes/$probe.md"
done

echo "[4/7] inheritance queries"
"${CRG_PY[@]}" scripts/parity/inherits/crg_inherits.py \
    "$OUT/work/inheritance/crg" "$OUT/inheritance/crg.json"
python3 scripts/parity/inherits/compare_inherits.py \
    "$OUT/inheritance/crg.json" "$OUT/work/inheritance/rust" \
    "$OUT/work/inheritance/crg" "$CG" > "$OUT/inheritance/compare.md"

echo "[5/7] change scenario 1: this repo HEAD~3"
clone_repo() {
    local src=$1 dst=$2 ref=$3
    git clone -q --shared "$src" "$dst"
    "${GIT[@]}" -C "$dst" checkout -q "$ref"
}
s1="$OUT/changes/rust-head3"
clone_repo "$ROOT" "$s1/crg" 3a786dc
clone_repo "$ROOT" "$s1/rust" 3a786dc
(cd "$s1/crg" && "${CRG[@]}" build > "$s1/crg-build.log" 2>&1)
(cd "$s1/rust" && "$CG" build . > "$s1/rust-build.log" 2>&1)
"${CRG_PY[@]}" scripts/parity/changes/crg_changes.py "$s1/crg" HEAD~3 \
    "$s1/crg.json" 2 > "$s1/crg-changes.log"
python3 scripts/parity/changes/compare_changes.py "$s1/crg.json" \
    "$s1/rust" "$s1/crg" HEAD~3 "$CG" 2 > "$s1/compare.md"

echo "[6/7] change scenario 2: synthetic edit of 2 CRG files (+ update timing)"
s2="$OUT/work/crg-synthetic"
clone_repo "$CRG_REPO" "$s2/crg" "$CRG_PIN"
clone_repo "$CRG_REPO" "$s2/rust" "$CRG_PIN"
(cd "$s2/crg" && "${CRG[@]}" build > "$s2/crg-build.log" 2>&1)
(cd "$s2/rust" && "$CG" build . > "$s2/rust-build.log" 2>&1)
edit_file() {
    python3 - "$1" "$2" <<'PY'
import sys
from pathlib import Path
path, symbol = Path(sys.argv[1]), sys.argv[2]
lines = path.read_text().splitlines(keepends=True)
for i, line in enumerate(lines):
    if line.startswith(f"def {symbol}("):
        lines.insert(i + 1, "    # P1 parity synthetic edit\n")
        break
else:
    raise SystemExit(f"{symbol} not found in {path}")
path.write_text("".join(lines))
PY
}
for side in crg rust; do
    edit_file "$s2/$side/code_review_graph/flows.py" compute_criticality
    edit_file "$s2/$side/code_review_graph/communities.py" _extract_keywords
    "${GIT[@]}" -C "$s2/$side" add -A
    "${GIT[@]}" -C "$s2/$side" commit -q -m 'parity: synthetic two-file edit'
done
t0=$(now)
(cd "$s2/crg" && "${CRG[@]}" update > "$s2/crg-update.log" 2>&1)
t1=$(now)
u0=$(now)
(cd "$s2/rust" && "$CG" update . > "$s2/rust-update.log" 2>&1)
u1=$(now)
for depth in 1 2; do
    dest="$OUT/changes/crg-synthetic-d$depth"
    mkdir -p "$dest"
    "${CRG_PY[@]}" scripts/parity/changes/crg_changes.py "$s2/crg" HEAD~1 \
        "$dest/crg.json" "$depth" > "$dest/crg-changes.log"
    python3 scripts/parity/changes/compare_changes.py "$dest/crg.json" \
        "$s2/rust" "$s2/crg" HEAD~1 "$CG" "$depth" > "$dest/compare.md"
done
python3 - "$OUT/timing.json" "$(elapsed "$t0" "$t1")" "$(elapsed "$u0" "$u1")" <<'PY'
import json
import sys
from pathlib import Path
Path(sys.argv[1]).write_text(json.dumps({
    "crg_update_s_debug": float(sys.argv[2]),
    "rust_update_s_debug": float(sys.argv[3]),
}, indent=2) + "\n")
PY

echo "[7/7] change scenario 3: CRG HEAD~1 (truncated impact)"
s3="$OUT/work/crg-head1"
clone_repo "$CRG_REPO" "$s3/crg" b891b6a
clone_repo "$CRG_REPO" "$s3/rust" b891b6a
(cd "$s3/crg" && "${CRG[@]}" build > "$s3/crg-build.log" 2>&1)
(cd "$s3/rust" && "$CG" build . > "$s3/rust-build.log" 2>&1)
dest="$OUT/changes/crg-head1"
mkdir -p "$dest"
"${CRG_PY[@]}" scripts/parity/changes/crg_changes.py "$s3/crg" HEAD~1 \
    "$dest/crg.json" 2 > "$dest/crg-changes.log"
python3 scripts/parity/changes/compare_changes.py "$dest/crg.json" \
    "$s3/rust" "$s3/crg" HEAD~1 "$CG" 2 > "$dest/compare.md"

python3 "$ROOT/scripts/parity/check_p1.py" --print "$OUT" > "$OUT/metrics.json"
echo "snapshot: $OUT"
echo "metrics:  $OUT/metrics.json"
