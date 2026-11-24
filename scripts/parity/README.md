# Parity harness

Compares the `graph.db` (schema v15) written by code-review-graph (CRG, Python) and by the Rust
`codegraph` binary on the same sources.

Prerequisites: `uv`, `git`, CRG checked out at `../code-review-graph` (run from source with
`uv run --frozen`; the `code-review-graph` on PATH is 2.3.7 / schema 9 and is never used), and
`cargo build -p codegraph-mcp-server --bin codegraph`. CRG runs with `--extra communities`
(igraph), so both sides detect communities with Leiden; without igraph CRG falls back to
grouping by directory (`community_detector = file`) and the ARI is meaningless.

Besides nodes and edges, the report compares the post-process tables: flows by entry-point
language (Jaccard of entry-point `qualified_name`s; for entries on both sides, mean/min Jaccard
of `flow_memberships` and mean/max |Δ criticality|) and communities (count per side,
`metadata.community_detector`, Adjusted Rand Index of `nodes.community_id` over the nodes both
sides assigned).

```bash
# Baseline over the per-language fixture corpus (corpus.json)
python3 scripts/parity/parity.py corpus --work $TMPDIR/parity --md docs/parity/baseline.md

# Python probe (nested classes, receivers, dead guards, references, submodule imports)
python3 scripts/parity/parity.py run scripts/parity/probes/python --work $TMPDIR/pyprobe --md $TMPDIR/pyprobe.md

# Python import probe (package/namespace/plain-module imports, ambiguous nested leaf).
# Result: nodes 1.000, CONTAINS 1.000, IMPORTS_FROM 1.000; CALLS 0.684 (resolved 0.786) and
# REFERENCES 0 are the documented gaps (dotted module path `q.lib.f()`, `mod.Thing().go()`,
# `fast` tier drops References).
python3 scripts/parity/parity.py run scripts/parity/probes/python-imports --work $TMPDIR/pyimports --md $TMPDIR/pyimports.md

# One arbitrary directory
python3 scripts/parity/parity.py run <src_dir> --work $TMPDIR/parity-run --md out.md --json out.json

# Compare two existing databases
python3 scripts/parity/parity.py compare crg.db <crg_root> rust.db <rust_root>

# Unit tests
python3 -m unittest discover -s scripts/parity -v
```
