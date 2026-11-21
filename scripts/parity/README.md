# Parity harness

Compares the `graph.db` (schema v15) written by code-review-graph (CRG, Python) and by the Rust
`codegraph` binary on the same sources.

Prerequisites: `uv`, `git`, CRG checked out at `../code-review-graph` (run from source with
`uv run --frozen`; the `code-review-graph` on PATH is 2.3.7 / schema 9 and is never used), and
`cargo build -p codegraph-mcp-server --bin codegraph`.

```bash
# Baseline over the per-language fixture corpus (corpus.json)
python3 scripts/parity/parity.py corpus --work $TMPDIR/parity --md docs/parity/baseline.md

# Python probe (nested classes, receivers, dead guards, references, submodule imports)
python3 scripts/parity/parity.py run scripts/parity/probes/python --work $TMPDIR/pyprobe --md $TMPDIR/pyprobe.md

# One arbitrary directory
python3 scripts/parity/parity.py run <src_dir> --work $TMPDIR/parity-run --md out.md --json out.json

# Compare two existing databases
python3 scripts/parity/parity.py compare crg.db <crg_root> rust.db <rust_root>

# Unit tests
python3 -m unittest discover -s scripts/parity -v
```
