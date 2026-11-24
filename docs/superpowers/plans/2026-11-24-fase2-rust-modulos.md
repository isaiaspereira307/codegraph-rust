# Rust Cross-File Module Resolution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve Rust `use` paths, scoped calls (`m::f()`, `Type::new()`) and `impl Trait for T` across files the way code-review-graph's `_resolve_rust_module_file` does, so IMPORTS_FROM, CALLS and IMPLEMENTS between Rust files match the CRG graph.

**Architecture:** The indexer reads `Cargo.toml` files from disk (crate `toml`, already a dependency of `codegraph-mcp`) into a `RustCrates` map. `ModuleIndex` (pure, path-based) takes that map and resolves Rust import text to an indexed file. `SymbolCatalog` uses the same resolver to bind scoped calls and trait paths to `<file>::Item` symbols. No new dependency, no `cargo metadata`, no new graph tables.

**Tech Stack:** Rust, `toml` 0.9, tree-sitter Rust extractor, the Python parity harness in `scripts/parity/`.

**Spec:** the "Design" section below (the user approved the approach in conversation on 2026-11-24, no separate spec file). Reference behaviour: `../code-review-graph/code_review_graph/parser.py` lines 22226-22520 (`_rust_dependency_specs`, `_rust_crate_layout`, `_rust_project_context`, `_rust_current_module_parts`, `_rust_module_file_for_parts`, `_resolve_rust_module_file`), 19551 (`_resolve_rust_type_target`), 22607 (`_resolve_rust_scoped_call`).

## Design

Approaches considered: (A) read `Cargo.toml` in the indexer and pass a map to `ModuleIndex` (chosen); (B) index `Cargo.toml` as nodes (pollutes the graph, CRG does not); (C) shell out to `cargo metadata` (slow, needs toolchain, not deterministic).

Behaviour to port (all from the CRG functions above):

1. Crate layout comes from the nearest `Cargo.toml` with a `[package]` at or above the importer: `[lib] path`, else `src/lib.rs`, else `src/main.rs`. No manifest or no layout: import stays text (CRG does the same; the current ancestor walk for `lib.rs`/`main.rs` goes away).
2. First segment `crate` = crate root module; `self` = importer's module; `super` (repeated) = parent modules; a name in the crate's path dependencies = that crate's root module (`lib.rs`); any other first segment must itself be a module of the crate root (`use util::helper` → `src/util.rs`), else text (`std`, `serde`, `Vec` never resolve).
3. Path dependencies: `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]` and `[target.*.*]` tables with a `path`, plus `workspace = true` entries inherited from the nearest `[workspace.dependencies]`. The alias is registered under both spellings (`my-crate`, `my_crate`). A dependency whose path leaves the project root, or has no crate layout, is ignored.
4. The last segment may be an item of the module reached; an earlier non-module segment, or a lone unanchored non-module segment, stops resolution (`None`).
5. Case: a module file is found only on an exact-case match. `ModuleIndex` already looks files up in an exact-case map, so `Glob` cannot match `glob.rs`; this plan only pins it with a test.
6. Scoped call `a::b::T::m` binds to `<file of a::b>::T.m` when the prefix minus its last segment resolves to the same file as the whole prefix; otherwise to `<file of prefix>::m`. Trait path in `impl Tr for T` binds to `<file>::Tr`. First segments are expanded through the file's `use` aliases (`use crate::m as n; n::f()`).

Goals: IMPORTS_FROM, CALLS, IMPLEMENTS Jaccard 1.000 on the new multi-file probe; this repo's IMPORTS_FROM Jaccard (0.721 on 2026-11-24) goes up, with the remaining gap explained. Non-goals: `mod` declarations, `#[path]`, re-exports (`pub use`), glob imports, macro-generated items, rust-analyzer, editing `Cargo.toml` triggering a re-resolve with unchanged sources (documented as a limit). Anti-goal: a Rust path that CRG leaves as text must not become an edge (no lexical guessing).

## Global Constraints

- Output deterministic: no HashMap iteration order may reach a result; use `BTreeMap`/sorted vectors in `RustCrates`.
- Crate layering: all new code in `codegraph-mcp` (indexer side) except the one parser attribute in `codegraph-parser`. No upward imports.
- New files start with two `// ABOUTME:` lines. `cargo fmt --all` (max_width 100) before each commit.
- `GRAPH_SHAPE_VERSION` in `crates/codegraph-parser/src/languages/crg_shape.rs` is bumped once (`crg-shape-10` → `crg-shape-11`) because the extractor output changes (Task 4).
- Unresolved paths keep their text (`crg_verbatim_target` behaviour untouched).
- Commit straight to `main`, no PR (project flow). Run `cargo clean` when the plan closes (memory: target/ hit 39G).

## Review Focus

- Importer has no `Cargo.toml` anywhere above it → every `crate::`/`self::`/`super::` import stays text (no panic, no guess).
- Malformed `Cargo.toml` (invalid TOML, `[package]` missing, `path` not a string) → that crate is skipped, indexing continues.
- `super::` climbing above the crate root, or `self::` in `lib.rs`/`main.rs`/`mod.rs` → `None`, no wrap-around.
- Dependency cycle between path crates (`a` depends on `b` depends on `a`) → resolution terminates (it never recurses into the dependency's own dependencies).
- Dependency path pointing outside the project root, or at a crate without `lib.rs` → ignored.
- Workspace member `Cargo.toml` using `workspace = true` while the workspace root has no matching `[workspace.dependencies]` entry → ignored.
- File named `Glob.rs` with a module path `glob` (and the reverse) → no match.

---

## File Structure

- Create `crates/codegraph-mcp/src/rust_crates.rs`: parse `Cargo.toml` text into crate layouts and dependency maps; pure functions plus one disk-reading constructor.
- Modify `crates/codegraph-mcp/src/module_index.rs`: Rust resolver rewritten on `RustCrates`.
- Modify `crates/codegraph-mcp/src/resolution.rs`: store `RustCrates`, expose `rust_path_target`, collect per-file `use` aliases.
- Modify `crates/codegraph-mcp/src/indexer.rs` (around line 1220 and `resolve_target` at ~2157): build and pass `RustCrates`; call `rust_path_target`.
- Modify `crates/codegraph-mcp/src/lib.rs`: `mod rust_crates;`.
- Modify `crates/codegraph-parser/src/languages/rust.rs` and `crg_shape.rs`: `Imports` edge carries the local alias; shape version bump.
- Create `scripts/parity/probes/rust-imports/`: multi-file, two-crate fixture.
- Modify `docs/parity/baseline.md`, `scripts/parity/README.md`, `PLANO.md`, `CLAUDE.md` (counts/notes only if they change).

---

### Task 1: Multi-file Rust probe and the "before" measurement

**Files:**
- Create: `scripts/parity/probes/rust-imports/Cargo.toml` (workspace, members `app`, `util-lib`)
- Create: `scripts/parity/probes/rust-imports/app/Cargo.toml` (package `app`, `[dependencies] util-lib = { path = "../util-lib" }`)
- Create: `scripts/parity/probes/rust-imports/app/src/{main.rs,config.rs,store/mod.rs,store/disk.rs,Glob.rs,glob.rs}`
- Create: `scripts/parity/probes/rust-imports/util-lib/Cargo.toml`, `util-lib/src/lib.rs`, `util-lib/src/text.rs`
- Modify: `docs/parity/baseline.md` (append a "Rust imports probe" section)

**Interfaces:**
- Produces: a probe dir that `parity.py run` accepts, exercising every item in the Design list: `use crate::config::load;`, `use self::disk::Disk;` inside `store/mod.rs`, `use super::config;` in `store/disk.rs`, `use util_lib::text::shout;`, `use store::Store;` (unanchored first segment), `config::load()` after `use crate::config;`, `Disk::open()` (type from another file), `impl Tr for Disk` where `trait Tr` lives in `config.rs` and is imported, `use std::fmt;` and `Vec::new()` (must stay text), and `Glob` vs `glob` module names that differ only by case (exercise item 5 on a case-sensitive FS).

- [ ] **Step 1: Write the fixture files.** Every function body is a call or empty; each file small (<30 lines). Keep names unique so CALLS edges are unambiguous except for one deliberate pair (`new` in two types) that CRG leaves as text.
- [ ] **Step 2: Run the harness against the current binary.** Build first: `cargo build -p codegraph-mcp-server --bin codegraph`. Run: `python3 scripts/parity/parity.py run scripts/parity/probes/rust-imports --work $TMPDIR/rsimports --md $TMPDIR/rsimports-before.md`. Expected: report completes; IMPORTS_FROM, CALLS or IMPLEMENTS Jaccard below 1.000 (this is the failing "test").
- [ ] **Step 3: Record the "before" numbers** (nodes, CONTAINS, CALLS, IMPORTS_FROM, IMPLEMENTS, TESTED_BY Jaccard, plus the list of edges only in CRG) in `docs/parity/baseline.md` under a new "Rust imports probe (antes)" heading. Also record this repo's whole-graph IMPORTS_FROM and CALLS Jaccard for the same CRG commit (`python3 scripts/parity/parity.py run . --work $TMPDIR/self --md $TMPDIR/self-before.md`), because Task 7 compares against it.
- [ ] **Step 4: Commit** `test(parity): rust-imports probe and baseline before module resolution`.

### Task 2: `RustCrates`, the Cargo.toml reader

**Files:**
- Create: `crates/codegraph-mcp/src/rust_crates.rs`
- Modify: `crates/codegraph-mcp/src/lib.rs` (add `mod rust_crates;` next to `mod module_index;`)
- Test: unit tests in `rust_crates.rs`

**Interfaces:**
- Produces:
  - `pub(crate) struct CrateLayout { pub dir: String, pub source_root: String, pub root_file: String, pub dependencies: BTreeMap<String, String> }`: `dir` is the crate directory, `dependencies` maps an alias (both `-` and `_` spellings) to the **dir** of another crate in the map. All paths use the spelling of the indexed file keys (project-relative, or absolute when the keys are absolute), normalized with no `.`/`..`.
  - `pub(crate) struct RustCrates { crates: BTreeMap<String, CrateLayout> }` keyed by `dir`.
  - `impl RustCrates { pub(crate) fn from_manifests(manifests: &BTreeMap<String, String>, files: &BTreeSet<String>) -> Self }`: `manifests` maps a manifest's directory to its `Cargo.toml` text; `files` is the set of indexed paths (used to decide whether `[lib] path`, `src/lib.rs` or `src/main.rs` exists, in that order, as CRG's `_rust_crate_layout`).
  - `pub(crate) fn discover(project_root: &Path, rust_files: &BTreeSet<String>) -> Self`: for each indexed `.rs` file walk its ancestor directories (stopping at the project root), read `<dir>/Cargo.toml` once per directory (relative dirs are joined to `project_root`), then call `from_manifests`. Unreadable files are skipped silently.
  - `pub(crate) fn crate_of(&self, file: &str) -> Option<&CrateLayout>`: the layout of the deepest crate whose `dir` is an ancestor of `file` (a `[package]` manifest only).
  - `pub(crate) fn dependency(&self, krate: &CrateLayout, alias: &str) -> Option<&CrateLayout>`.
- Workspace inheritance: a `workspace = true` dependency takes its table from the nearest ancestor manifest with `[workspace.dependencies]`, `path` resolved relative to that workspace manifest's dir (CRG `workspace_base`).

- [ ] **Step 1: Write failing tests** in `rust_crates.rs`:
  - `layout_prefers_lib_path_then_lib_rs_then_main_rs` (three manifests; assert `root_file`).
  - `path_dependencies_register_both_spellings` (`my-crate = { path = "../my-crate" }` → aliases `my-crate` and `my_crate`).
  - `target_specific_and_dev_dependencies_count` (`[target.'cfg(unix)'.dependencies]` and `[dev-dependencies]`).
  - `workspace_dependencies_are_inherited` (`dep = { workspace = true }` + root `[workspace.dependencies] dep = { path = "crates/dep" }`).
  - `broken_manifests_and_outside_paths_are_skipped` (invalid TOML; `path = "../../outside"`; dep without layout) asserting the map still holds the valid crates.
  - `crate_of_picks_the_deepest_package` (nested crate inside another).
  - `dependency_cycles_do_not_recurse` (`a`↔`b`): both layouts present, `dependency` returns the direct neighbour only.
- [ ] **Step 2: Run** `cargo test -p codegraph-mcp rust_crates` → FAIL (module missing).
- [ ] **Step 3: Implement** the types above with `toml::Table` parsing (`toml::from_str::<toml::Table>`); sort everything via `BTreeMap`; normalize paths with the same collapse-`..` rule as `module_index::normalize` (make that function `pub(crate)` and reuse it).
- [ ] **Step 4: Run** `cargo test -p codegraph-mcp rust_crates` → PASS.
- [ ] **Step 5: Commit** `feat(mcp): read Cargo.toml crate layouts and path dependencies`.

### Task 3: `ModuleIndex` resolves Rust through `RustCrates`

**Files:**
- Modify: `crates/codegraph-mcp/src/module_index.rs` (`ModuleIndex`, `fn rust` at ~193, tests at ~272 and ~377)

**Interfaces:**
- Consumes: `RustCrates::crate_of`, `RustCrates::dependency`, `CrateLayout` (Task 2).
- Produces: `ModuleIndex::with_rust_crates(self, crates: RustCrates) -> Self`; `ModuleIndex::resolve` signature unchanged. `fn rust(&self, importer: &str, text: &str) -> Option<&str>` now follows Design items 1-5. Module file lookup under a source root, for module parts `p`: `<source_root>/<p joined>.rs`, then `<source_root>/<p joined>/mod.rs`; no parts → `root_file`. Current module parts of the importer: its path below `source_root`, dropping the final `lib.rs`/`main.rs`/`mod.rs` filename and the `.rs` suffix otherwise (`_rust_current_module_parts`). Result must lie inside the importer's crate dir or the dependency's dir.

- [ ] **Step 1: Update the shared test index** so the existing `rust_paths_anchor_at_the_crate_root_or_the_current_module` supplies a manifest for `crate/` via `RustCrates::from_manifests` (the old no-manifest walk is intentionally removed). Add failing tests:
  - `rust_unanchored_first_segment_is_a_crate_root_module` (`index.resolve(lib, "m::f") == Some("crate/src/m.rs")`; `resolve(lib, "Vec::new") == None`; `resolve(lib, "serde::Serialize") == None`).
  - `rust_path_dependency_resolves_to_its_lib_rs` (`use other_lib::text::shout` from `app/src/main.rs` → `other-lib/src/text.rs`; `use other_lib::X` → `other-lib/src/lib.rs`; `other_lib` and `other-lib` spellings both work).
  - `rust_without_a_manifest_stays_text` (importer in a dir tree with no manifest, `crate::m` → `None`).
  - `rust_super_above_the_root_and_self_in_root_files` (`super::x` from `lib.rs` → `None`; `self::m` from `lib.rs` resolves `m` under the root).
  - `rust_module_files_match_case_exactly` (index holds `crate/src/Glob.rs` only; `crate::glob` → `None`; with only `glob.rs`, `crate::Glob` → `None`).
- [ ] **Step 2: Run** `cargo test -p codegraph-mcp module_index` → FAIL.
- [ ] **Step 3: Implement** `with_rust_crates` and rewrite `fn rust` per Design. The dependency branch switches `source_root`/`root_file` to the dependency's layout and clears the current module parts; it never recurses into the dependency's own dependencies.
- [ ] **Step 4: Run** `cargo test -p codegraph-mcp module_index` → PASS (all language tests, Python/script/Java unchanged).
- [ ] **Step 5: Commit** `feat(mcp): resolve Rust use paths through Cargo layouts and path dependencies`.

### Task 4: Wire `RustCrates` into the indexer; imports become File edges

**Files:**
- Modify: `crates/codegraph-mcp/src/resolution.rs` (`SymbolCatalog::new` ~line 119 area, new setter)
- Modify: `crates/codegraph-mcp/src/indexer.rs` (~line 1220, where `SymbolCatalog::new(&nodes)` is built)
- Modify: `crates/codegraph-parser/src/languages/crg_shape.rs` (`GRAPH_SHAPE_VERSION`)
- Test: `crates/codegraph-mcp/tests/` (new `rust_imports_test.rs`) or an indexer unit test following the existing ones near `indexer.rs:2700`

**Interfaces:**
- Produces: `SymbolCatalog::set_rust_crates(&mut self, crates: RustCrates)` which rebuilds `self.modules` as `ModuleIndex::new(files).with_rust_crates(crates)`. The indexer calls it right after `SymbolCatalog::new(&nodes)` and before `retain_python_imports`/`link_imports`, with `RustCrates::discover(<project root the indexer already holds>, &rust files among catalog.file_nodes keys)`.
- Version: `GRAPH_SHAPE_VERSION` becomes `crg-shape-11`.

- [ ] **Step 1: Write the failing test**: create a temp project with the Task 1 shape in miniature (two crates, `use crate::config::load;`, `use util_lib::text::shout;`), run the real indexer entry used by `reconciliation_test.rs`/`integration_tests.rs`, read the SQLite store, assert `edges(kind='IMPORTS_FROM')` contains `File(app/src/main.rs) → File(app/src/config.rs)` and `→ File(util-lib/src/text.rs)`, and that `use std::fmt;` stays a text target.
- [ ] **Step 2: Run** the test → FAIL (imports stay text).
- [ ] **Step 3: Implement** `set_rust_crates`, the indexer call, and the shape-version bump. Run `cargo test -p codegraph-parser` to catch tests that assert the version string.
- [ ] **Step 4: Run** the new test + `cargo test -p codegraph-mcp` → PASS.
- [ ] **Step 5: Commit** `feat(mcp): Rust imports bind to File nodes across crates`.

### Task 5: `Imports` edges carry the local alias

**Files:**
- Modify: `crates/codegraph-parser/src/languages/rust.rs` (the `use_declaration` arm at ~line 199 and `flatten_use` at ~929)
- Modify: `crates/codegraph-parser/src/languages/crg_shape.rs` (new `pub const RUST_ALIAS_ATTR: &str = "crg_rust_alias";`)

**Interfaces:**
- Produces: every Rust `Imports` edge has `metadata["crg_rust_alias"]` = the local name the `use` binds (`use x::Y as Z` → `Z`; `use a::b::C` → `C`; `use a::{self}` → `a`). `flatten_use` already returns `(local_name, path)` pairs; the arm currently drops the first element. Edge `to` text stays unchanged.

- [ ] **Step 1: Write the failing test** next to `use_trees_flatten_like_crg`: `use_edges_record_the_local_alias` asserting `(to, alias)` pairs for `use x::Y as Z; use a::{b, c::D, self as e}; use std::fmt;` → `("x::Y","Z")`, `("a::b","b")`, `("a::c::D","D")`, `("a::self","e")`, `("std::fmt","fmt")`.
- [ ] **Step 2: Run** `cargo test -p codegraph-parser use_edges_record_the_local_alias` → FAIL.
- [ ] **Step 3: Implement** by keeping the pair list and inserting `RUST_ALIAS_ATTR` in each edge's metadata.
- [ ] **Step 4: Run** `cargo test -p codegraph-parser` → PASS.
- [ ] **Step 5: Commit** `feat(parser): record the local alias on Rust import edges`.

### Task 6: Scoped calls and `impl Trait for T` bind across files

**Files:**
- Modify: `crates/codegraph-mcp/src/resolution.rs` (collect aliases in `link_imports`; add `rust_path_target`)
- Modify: `crates/codegraph-mcp/src/indexer.rs` (`resolve_target` ~line 2157: call `rust_path_target` before `catalog.exact` for Rust edges)

**Interfaces:**
- Consumes: `RUST_ALIAS_ATTR` (Task 5), `ModuleIndex::resolve` with Rust crates (Tasks 3-4).
- Produces: `SymbolCatalog::rust_path_target(&self, edge: &EdgeRelationship) -> Option<NodeId>`. Applies only when the importer file ends in `.rs`, the edge type is `Calls` or `Implements`, and `edge.to` contains `::`. `link_imports` additionally stores, per importing file, `alias → import text` for Rust `Imports` edges (`rust_aliases: HashMap<String, HashMap<String, String>>`).
  - Calls (port of `_resolve_rust_scoped_call`): split `to` into `prefix::method`; first segment expanded through the file's alias map; resolve the whole prefix to a file `F` via `modules.resolve`; resolve `prefix[..len-1]` to `P`; if `P == F` and the last prefix segment is not `crate`/`self`/`super` the target alias is `F::<last prefix segment>.<method>`, else `F::<method>`. Return the symbol with exactly that qualified name (`qualified_symbol`), else `None` (the edge then keeps the existing path; unresolved stays text).
  - Implements (port of `_resolve_rust_type_target`): a lone first segment found in the alias map expands to the import text; then resolve the full path to file `F` and bind `F::<last segment>`; otherwise resolve the written path directly.
- `resolve_target` returns `(catalog.rust_path_target(edge), ("import", 1.0))` when it yields `Some`; when it yields `None` the existing verbatim/exact logic runs unchanged.

- [ ] **Step 1: Write failing tests** in `resolution.rs` (style of the tests near line 500-750, building nodes and edges by hand) and one end-to-end assertion in the Task 4 test project:
  - `rust_scoped_call_binds_to_the_function_in_the_module_file` (`config::load` after `use crate::config;` → `app/src/config.rs::load`).
  - `rust_associated_call_binds_to_the_type_in_the_module_file` (`Disk::open` after `use crate::store::disk::Disk;` → `.../disk.rs::Disk.open`).
  - `rust_alias_import_is_expanded` (`use crate::config as cfg; cfg::load()`).
  - `rust_trait_impl_binds_to_the_trait_in_another_file` (`impl Tr for Disk`, `Tr` imported from `config.rs` → `IMPLEMENTS` edge to `config.rs::Tr`).
  - `rust_std_and_unknown_paths_stay_text` (`Vec::new`, `fmt::Display`, `serde::Serialize` → `None`).
  - `rust_dependency_crate_call_binds_into_the_other_crate` (`util_lib::text::shout()`).
- [ ] **Step 2: Run** `cargo test -p codegraph-mcp rust_` → FAIL.
- [ ] **Step 3: Implement** `rust_path_target` and the alias collection; keep `link_imports` behaviour for imported-file reachability unchanged.
- [ ] **Step 4: Run** `cargo test -p codegraph-mcp` and `cargo test -p codegraph-parser` → PASS.
- [ ] **Step 5: Commit** `feat(mcp): bind Rust scoped calls and trait impls across files`.

### Task 7: Measure, document, close

**Files:**
- Modify: `docs/parity/baseline.md`, `scripts/parity/README.md`, `PLANO.md`, `CLAUDE.md` (only if a stated fact changed)

- [ ] **Step 1: Re-run the harness** on the probe (`rust-imports`), the corpus (`python3 scripts/parity/parity.py corpus --work $TMPDIR/parity --md $TMPDIR/corpus.md`) and this repo (`... run . --work $TMPDIR/self --md $TMPDIR/self-after.md`). Expected: probe IMPORTS_FROM, CALLS, IMPLEMENTS at 1.000 (or each remaining edge listed with its cause); corpus Rust row still 1.00 everywhere; this repo's IMPORTS_FROM above 0.721. If a probe number is below 1.000, either fix the cause or document it in baseline.md under "Fica para depois".
- [ ] **Step 2: Run the full gates**: `cargo test --workspace --all-features --no-fail-fast -- --skip test_watcher_detects_create_modify_delete_with_debounce --skip test_watcher_delete_event --skip test_file_change_detection --skip test_file_monitoring_only --skip test_integration_test_suite` and `cargo clippy -p codegraph-mcp -p codegraph-parser --all-targets -- -D warnings` (strict workspace clippy is known to fail in `codegraph-core`; do not claim a workspace pass).
- [ ] **Step 3: Re-run the review-parity check** (`docs/parity/changes.md`, scripts in `scripts/parity/changes/`) for the Rust scenario and record the new `impacted` Jaccard next to the old 0.897.
- [ ] **Step 4: Update the docs**: baseline.md "depois" numbers; README line for the new probe; in `PLANO.md` mark Next Step 1 done, replace the Rust "Fica para depois" text with what remains (`pub use` re-exports, glob imports, `mod` declarations, receiver typing, Cargo.toml edits not re-resolving unchanged sources); note the `crg-shape-11` bump in Fase 2.5 item 1.
- [ ] **Step 5: Commit** `docs: close Rust cross-file module resolution`, then `cargo clean`.

---

## Self-Review

- Spec coverage: Design items 1-3 → Tasks 2-3; item 4 (final-segment rule) → Task 3; item 5 (case) → Task 3 test (no new code, stated in Design); item 6 → Tasks 5-6; measurement and docs → Tasks 1 and 7; shape-version bump → Task 4.
- Type consistency: `RustCrates`, `CrateLayout`, `set_rust_crates`, `with_rust_crates`, `rust_path_target`, `RUST_ALIAS_ATTR` are used with the same names in every task that mentions them.
- Known risk, not covered by tasks: editing only `Cargo.toml` leaves old edges until a source file changes (non-goal, documented in Task 7).

## Resultado (2026-11-24)

- Probe `rust-imports`: CALLS, IMPORTS_FROM, IMPLEMENTS 1.000 (from 0.267 / 0.500 / 1.000). This repo: IMPORTS_FROM 0.729 → 0.991 (resolved 1.000; the rest is SQL), CALLS 0.931 → 0.964. Phase 3 Rust scenario `impacted` 0.897 → 0.948. Numbers and remaining edges: `docs/parity/baseline.md` "Rust imports probe (depois)".
- Measuring found three defects, fixed in separate commits: `./`-prefixed file keys never matched a crate root, so no Rust import resolved under `codegraph index .` (`3b7ce1c`); a scoped call to a missing item did not settle to the plain name as CRG does (`c897a49`); `use` inside `mod { }`/function bodies set aliases CRG ignores (`3840ab1`).
- `GRAPH_SHAPE_VERSION` lives in `crates/codegraph-mcp/src/indexer.rs`, not `crg_shape.rs`; it went to `crg-shape-11` (Task 4) and then `crg-shape-12` (`3840ab1`, parser output changed).
