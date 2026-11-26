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

# Rust import probe (Cargo workspace: crate::/self::/super::, path dependency, `use` aliases,
# scoped calls, `impl Tr for T` across files, Glob.rs/glob.rs case). Result: nodes, CALLS,
# IMPORTS_FROM and IMPLEMENTS 1.000; CONTAINS 0.737 (Rust-only Directory hierarchy).
python3 scripts/parity/parity.py run scripts/parity/probes/rust-imports --work $TMPDIR/rsimports --md $TMPDIR/rsimports.md

# Inheritance probe (Python, TS, Java, Go: a base with two subclasses, an ambiguous base defined
# twice, external bases, Java `Shape<Integer>`, Python `shapes.Shape`). Compares query_graph
# inheritors_of per type node and get_impact_radius per file (depth 1, 2) with CRG; result in
# docs/parity/baseline.md, "INHERITS no GraphQuery". Index both copies first (as run_pair does),
# then:
uv run --frozen --project ../code-review-graph python scripts/parity/inherits/crg_inherits.py <crg_root> crg.json
python3 scripts/parity/inherits/compare_inherits.py crg.json <rust_root> <crg_root> target/debug/codegraph

# One arbitrary directory
python3 scripts/parity/parity.py run <src_dir> --work $TMPDIR/parity-run --md out.md --json out.json

# Compare two existing databases
python3 scripts/parity/parity.py compare crg.db <crg_root> rust.db <rust_root>

# Unit tests
python3 -m unittest discover -s scripts/parity -v
```

## P1 baseline and regression check

`p1_snapshot.sh` runs the corpus, the probes (python, python-imports, rust-imports,
inheritance, typescript-js, go-package, java-cross-file, java-framework and one
`<language>-imports` probe each for Swift, C/C++, C#, Ruby, PHP, Kotlin and Dart) and the three
change scenarios in one go. It pins the CRG checkout at
`bbdda4b` (a throwaway clone in `<out-dir>/work/crg-repo`), builds the Rust CLI and writes only
to `<out-dir>`; the frozen limits live in `docs/parity/p1-baseline.md`.

```bash
bash scripts/parity/p1_snapshot.sh /tmp/opencode/parity-p1
python3 scripts/parity/check_p1.py /tmp/opencode/parity-p1
python3 scripts/parity/check_p1.py --print /tmp/opencode/parity-p1  # all measured floors/values
```

`check_p1.py` exits 1 if any metric with a frozen floor falls (tolerance 0.0006 for the
three-decimal Markdown tables), and ignores improvements. Timing in `timing.json` is recorded
for information; the release/debug target is checked separately by `docs/parity/changes.md`.

## LSP tier evaluation

`docs/parity/lsp-tier.md` records the `fast` × `balanced` × `full` comparison. The harness does
not take a tier flag; set it in the environment for the Rust side (CRG is unaffected):

```bash
CODEGRAPH_INDEX_TIER=balanced python3 scripts/parity/parity.py run <src> \
  --work $TMPDIR/tier-balanced --crg-repo ../code-review-graph \
  --codegraph target/debug/codegraph --md $TMPDIR/tier-balanced.md
```

`balanced`/`full` require `pyright-langserver`, `typescript-language-server`, `gopls`, `jdtls`
and `rust-analyzer` on PATH; missing tools abort the index in preflight. To time only the Rust
`build`, point `--codegraph` at a wrapper that forwards to the real binary and appends the
elapsed time of `index` invocations to a log. The measured conclusion was to keep `fast` as the
default tier.
