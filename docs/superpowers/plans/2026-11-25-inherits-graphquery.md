# INHERITS/IMPLEMENTS in GraphQuery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the graph tools follow inheritance: a base class or interface that the source names as text (`Base`, `unittest.TestCase`) is bound, at read time, to the node it names, so `get_impact_radius`, `traverse_graph` and a new `query_graph` pattern `inheritors_of` reach subclasses and implementers (TS/JS, Go, Java, Python).

**Architecture:** `INHERITS`/`IMPLEMENTS` edges stay as text in SQLite, as code-review-graph (CRG) writes them (the DB stays readable by both tools). `Snapshot::load` (in-memory petgraph copy, `crates/codegraph-graph/src/graph_query/snapshot.rs`) currently skips every edge whose target is not a node's `qualified_name`. A new module derives extra petgraph edges for heritage edges from the bare name, with a deterministic tie-break, and records the ambiguous ones. Everything downstream (`is_dependency`, `neighbours`, `expand`) already includes `INHERITS`/`IMPLEMENTS`, so impact and traverse need no change beyond tests.

**Tech Stack:** Rust, `rusqlite`, `petgraph`, the Python parity harness in `scripts/parity/`.

**Spec:** the "Design" section below (approach approved in conversation on 2026-11-24 as Next Step 2 of `PLANO.md`; no separate spec file). Reference behaviour: `../code-review-graph/code_review_graph/tools/query.py` lines 1195-1210 and 1581-1612 (`inheritors_of`, bare-name fallback, `ambiguous_base`), `graph.py:2294` (`iter_edges_by_target_name`), `constants.py:260-270` (`IMPACT_EDGE_DIRECTIONS`: `INHERITS`/`IMPLEMENTS` impact flows target to source), `graph.py:210` (`_compatible_edge_languages`).

## Design

Approaches considered: (A) derive edges in `Snapshot::load` and leave SQLite alone (chosen); (B) resolve in the indexer and write `target_qualified` (breaks CRG parity: CRG keeps the text, and `crg_verbatim` behaviour from Phases 2.x stays); (C) per-query SQL lookup by name (repeats work on every BFS step, and `expand`/impact work on petgraph).

Rules for a heritage edge (`kind` in `INHERITS`, `IMPLEMENTS`) whose `target_qualified` is not a node's `qualified_name`:

1. **Name:** the target text's last segment, splitting on `.` and `::` (`unittest.TestCase` → `TestCase`, `a.b.Base` → `Base`), ignoring balanced type-argument groups `<…>` anywhere and trimming whitespace (`Shape<Integer>` and `Shape <Integer>` → `Shape`, `Outer<T>.Inner` → `Inner`, `Foo<A, B<C>>` → `Foo`). Empty text (`<T>`, `Foo::<T>`) never binds. *(Amended in Task 4: type arguments were added in Task 3, and cutting them anywhere rather than at the first `<` in Task 4.)*
2. **Candidates:** nodes whose `name` equals that segment and whose `kind` is one of `Class`, `Interface`, `Type`, `Struct`, `Enum`, `Trait` (CRG's list), in a language compatible with the source node's language. Compatible families: `{javascript, typescript, tsx}`, `{c, cpp}`; every other language only matches itself. A candidate equal to the source node is dropped (no self edge).
3. **Tie-break by tiers:** (a) candidates in the source's own file; (b) candidates in a file the source's file imports (an `IMPORTS_FROM` edge `File → File` already in the snapshot); (c) all candidates in the repository. The **first tier that has any candidate decides**: one candidate binds; several are **ambiguous** and there is no fall-through to a later tier. An ambiguous edge adds no edge and is counted once on **each candidate node** of the deciding tier (`Snapshot::ambiguous_heritage: HashMap<NodeIndex, usize>`), so `inheritors_of` on one of them reports it in `unresolved`; a base with zero candidates is not counted there (CRG instead reports every candidate with `inferred_by: "bare_name"`; this plan prefers no edge to a wrong impact set, see Review Focus). *(Amended in Task 4 to match the code: the plan first said "keyed by bare name" and "one candidate per tier".)*
4. **Zero candidates** (external bases: `object`, `Generic`, `React.Component`) stay unresolved and count as before.
5. The derived edge keeps the original kind (`INHERITS`/`IMPLEMENTS`) and direction (child → base).
6. Determinism: candidates sorted by `(file_path, qualified_name)`; no `HashMap` iteration order reaches a result.

Goals: `inheritors_of` and impact radius reach subclasses in all four languages on a new probe; impact/inheritors sets agree with CRG on the probe, with each divergence explained. Non-goals: `AnalysisGraph` (flows, communities, risk, hubs; it has its own loader in `crates/codegraph-graph/src/analysis/mod.rs`), `OVERRIDES`, structural Go interface satisfaction, dotted-qualifier matching beyond the last segment, any DB write, any indexer or parser change. Anti-goal: an external or ambiguous base must not become an edge.

## Global Constraints

- No SQLite schema or row change, no `GRAPH_SHAPE_VERSION` bump (read side only).
- Crate layering: code goes in `codegraph-graph` (resolution, query), `codegraph-mcp-tools` (schema, executor) and `codegraph-mcp-server` (MCP and CLI help text). No upward imports.
- New files start with two `// ABOUTME:` lines. Format only touched files with `rustfmt --edition 2024 <file>` (not `cargo fmt --all`).
- Output deterministic. Strict workspace clippy fails in `codegraph-core`; use non-strict clippy on touched crates and do not claim a workspace pass.
- Commit straight to `main`, no PR. Run `cargo clean` when the plan closes (memory: `target/` reached 39G). Stage by explicit path.

## Review Focus

- A base name defined in two files and neither the source's file nor an imported one → no edge, counted as ambiguous.
- External base (`unittest.TestCase`, `object`, `Generic`) → no edge, no panic.
- `class A(A)` or a cycle `A → B → A` → no self edge, impact/expand terminate.
- A function or variable named like the base (kind not in the type list) → never a candidate.
- Python `Base` and a TS `Base` in the same repo → not cross-bound (language families).
- Heritage text equal to a real `qualified_name` already binds through the existing path and is not duplicated.
- Large graph: candidates indexed once by name, not scanned per edge.

## File Structure

- Create `crates/codegraph-graph/src/graph_query/heritage.rs`: pure resolution of text heritage targets over the loaded nodes.
- Modify `crates/codegraph-graph/src/graph_query/snapshot.rs` (`Snapshot::load`, new field) and `mod.rs` (`mod heritage;`).
- Modify `crates/codegraph-graph/src/graph_query/query.rs`: `QueryPattern::InheritorsOf`.
- Modify `crates/codegraph-mcp-tools/src/graph_tool_schemas.rs` (~411-420), `graph_tool_executor.rs` (~557), `crates/codegraph-mcp-server/src/official_server.rs` (~59, ~421), `bin/codegraph.rs` (~193): pattern list and help text.
- Tests: `crates/codegraph-graph/tests/graph_query_inherits_test.rs` (new), an end-to-end test in `crates/codegraph-mcp/tests/reconciliation_test.rs`.
- Create `scripts/parity/probes/inheritance/`: four small files (Python, TS, Java, Go), a base each with two subclasses plus one ambiguous and one external base.
- Modify `docs/parity/baseline.md`, `scripts/parity/README.md`, `PLANO.md`, `CLAUDE.md` (only if a stated fact changes).

---

### Task 1: Heritage resolution in the snapshot

**Files:**
- Create: `crates/codegraph-graph/src/graph_query/heritage.rs`
- Modify: `crates/codegraph-graph/src/graph_query/snapshot.rs` (the edge loop in `Snapshot::load`, struct `Snapshot`), `crates/codegraph-graph/src/graph_query/mod.rs` (`mod heritage;`)
- Test: `crates/codegraph-graph/tests/graph_query_inherits_test.rs`

**Interfaces:**
- Produces: `pub(crate) struct HeritageNode<'a> { pub qualified: &'a str, pub name: &'a str, pub kind: Option<&'a str>, pub file_path: &'a str, pub language: Option<&'a str> }` and `pub(crate) fn resolve_heritage(nodes: &[HeritageNode<'_>], heritage: &[(usize, String)], imported_files: &dyn Fn(usize) -> Vec<String>) -> HeritageResolution` where `heritage` pairs a source node index with its text target, and `HeritageResolution { pub bound: Vec<(usize, usize)>, pub ambiguous: Vec<(usize, String)> }` (source index → base index; ambiguous source with its text). `imported_files(source_ix)` returns the file paths the source's file imports.
- `Snapshot` gains `pub ambiguous_heritage` mapping to the number of ambiguous heritage edges (used by Task 2). *(As built: `HashMap<NodeIndex, usize>` keyed by each ambiguous candidate node, see Design rule 3.)*
- Rules 1-6 of the Design.

- [ ] **Step 1: Write failing unit tests** in `heritage.rs` (`#[cfg(test)]`): `last_segment_of_dotted_and_scoped_names`, `same_file_candidate_wins`, `imported_file_candidate_wins_over_repo_wide_duplicates`, `unique_repo_candidate_binds`, `two_candidates_without_a_tie_break_are_ambiguous`, `external_base_stays_unbound`, `function_named_like_the_base_is_not_a_candidate`, `self_edge_is_dropped`, `js_and_ts_share_a_family_but_python_does_not`. Each asserts `bound`/`ambiguous` exactly.
- [ ] **Step 2: Run** `cargo test -p codegraph-graph --lib heritage` → FAIL (module missing).
- [ ] **Step 3: Implement** `heritage.rs` (candidates indexed once in a `BTreeMap<&str, Vec<usize>>` by name, per Review Focus) and wire it into `Snapshot::load`: collect heritage edges whose target is not in `by_qn`, call `resolve_heritage`, add the derived petgraph edges (kind preserved) and fill `ambiguous_heritage`. `imported_files` reads `IMPORTS_FROM` edges already loaded whose source is the source's `File` node. Update the module doc comment in `snapshot.rs` ("edges to unknown targets are skipped" is no longer true for heritage).
- [ ] **Step 4: Write the integration test** `graph_query_inherits_test.rs` in the style of `graph_query_test.rs` (fixture helpers `node`/`fixture`; add an `inherits` edge helper that writes a text target): `impact_radius_reaches_a_subclass_through_a_text_base` (`get_impact_radius` on the base id returns the subclass at depth 1), `traverse_graph_follows_extends_in_reverse`, `ambiguous_base_adds_no_edge`, `external_base_adds_no_edge`, `existing_qualified_target_is_not_duplicated`.
- [ ] **Step 5: Run** `cargo test -p codegraph-graph` → PASS.
- [ ] **Step 6: Commit** `feat(graph): bind text INHERITS/IMPLEMENTS targets by name when loading the snapshot`.

### Task 2: `query_graph` pattern `inheritors_of`

**Files:**
- Modify: `crates/codegraph-graph/src/graph_query/query.rs`, `crates/codegraph-mcp-tools/src/graph_tool_schemas.rs`, `crates/codegraph-mcp-tools/src/graph_tool_executor.rs`, `crates/codegraph-mcp-server/src/official_server.rs`, `crates/codegraph-mcp-server/src/bin/codegraph.rs`
- Test: `crates/codegraph-graph/tests/graph_query_inherits_test.rs`, and the existing executor/MCP tests that assert pattern lists (grep `importers_of or tests_for`).

**Interfaces:**
- Consumes: `Snapshot::ambiguous_heritage` (Task 1).
- Produces: `QueryPattern::InheritorsOf` (`"inheritors_of"`): neighbours over incoming `INHERITS` **and** `IMPLEMENTS` (union, distinct, sorted by name, as CRG). `QueryGraphResult.unresolved` for this pattern = `ambiguous_heritage[target]` (heritage edges ambiguous between this node and other candidates; as built, see Design rule 3); for the other patterns it is unchanged. The tool schema and help text list `inheritors_of`.

- [ ] **Step 1: Write failing tests**: `inheritors_of_lists_subclasses_and_implementers` (one `INHERITS`, one `IMPLEMENTS` child of the same target, both returned, sorted), `inheritors_of_counts_ambiguous_bases_as_unresolved`, `inheritors_of_on_a_leaf_class_is_empty`; update the schema/executor tests that enumerate patterns to expect the new name.
- [ ] **Step 2: Run** `cargo test -p codegraph-graph --test graph_query_inherits_test` and `cargo test -p codegraph-mcp-tools` → FAIL.
- [ ] **Step 3: Implement** the variant (`parse`, `name`, a new arm handling two edge kinds; `edge()` returns one kind, so give `InheritorsOf` its own branch in `query_graph`) and update every pattern list in the files above, including `Unknown pattern` error text.
- [ ] **Step 4: Run** `cargo test -p codegraph-graph`, `cargo test -p codegraph-mcp-tools`, `cargo test -p codegraph-mcp-server --lib` → PASS (the `every_graph_tool_is_exposed_over_mcp` test must stay green).
- [ ] **Step 5: Commit** `feat(graph): query_graph inheritors_of`.

### Task 3: End to end through the indexer

**Files:**
- Test: `crates/codegraph-mcp/tests/reconciliation_test.rs` (extend, using the existing `index_files`/store helpers; it is env-gated like its neighbours, so check the run really executed)
- Create: `scripts/parity/probes/inheritance/` (see File Structure)

**Interfaces:**
- Consumes: Tasks 1-2.
- Produces: the probe, reused by Task 4. Per language one base, two subclasses, one subclass of an external base (`unittest.TestCase` / `React.Component` / `Exception`-like / none for Go), and one base name defined twice in two languages-compatible files (ambiguous case), kept small (<30 lines per file).

- [ ] **Step 1: Write the probe files**, then a failing e2e test `inherits_text_targets_bind_across_files_and_languages`: index a temp copy of the probe, open `GraphQuery`, assert `inheritors_of` on each base returns both subclasses by name, the external-base subclass is not returned for any in-repo node, and the duplicated base reports the ambiguity in `unresolved`.
- [ ] **Step 2: Run** the test → it must reach the assertions (not skip) and fail only if Task 1/2 were wrong; fix the code, not the test, if so.
- [ ] **Step 3: Run** `cargo test -p codegraph-mcp --test reconciliation_test` → PASS.
- [ ] **Step 4: Commit** `test(mcp): inheritance probe and end-to-end inheritors_of`.

### Task 4: Measure against CRG, document, close

**Files:**
- Modify: `docs/parity/baseline.md`, `scripts/parity/README.md`, `PLANO.md`, `CLAUDE.md` (only if a stated fact changed: tool pattern lists)

- [ ] **Step 1: Measure.** Build `cargo build -p codegraph-mcp-server --bin codegraph`; index `scripts/parity/probes/inheritance` with the Rust binary and with CRG (`uv run --frozen --project ../code-review-graph`, as `scripts/parity/parity.py` does). For each base class compare `inheritors_of` and `get_impact_radius` (depth 1 and 2) between the two tools. Record Jaccard per language and every divergence with its cause (expected: CRG lists all candidates of an ambiguous base with `inferred_by: bare_name`; this plan adds none). Also re-run `python3 scripts/parity/parity.py corpus` to confirm no node/edge regression (read-side change, expect identical numbers). Use scratch dirs under the session scratchpad.
- [ ] **Step 2: Full gates:** `cargo test --workspace --all-features --no-fail-fast -- --skip test_watcher_detects_create_modify_delete_with_debounce --skip test_watcher_delete_event --skip test_file_change_detection --skip test_file_monitoring_only --skip test_integration_test_suite`; non-strict clippy on `codegraph-graph`, `codegraph-mcp-tools`, `codegraph-mcp-server` for lines this plan added. State plainly that strict workspace clippy still fails in `codegraph-core`.
- [ ] **Step 3: Docs.** `baseline.md` section "INHERITS no GraphQuery" with the numbers; README line for the probe; `PLANO.md`: mark Next Step 2 done, update the TypeScript note that says the tools do not follow inheritance, list what remains (`AnalysisGraph` ignores text heritage, ambiguous bases add no edge unlike CRG, structural Go interfaces, dotted-qualifier matching, `OVERRIDES`); `CLAUDE.md` if the `query_graph` pattern list is stated there.
- [ ] **Step 4: Commit** `docs: close INHERITS in GraphQuery`, then `cargo clean`.

---

## Self-Review

- Spec coverage: Design rules 1-6 → Task 1; `inheritors_of` and the ambiguity count → Task 2; four-language behaviour and external/ambiguous bases end to end → Task 3; CRG parity and docs → Task 4. Review Focus lines map to Task 1 tests (ambiguity, external, self/cycle, kind filter, language family, duplicate qualified target) and Task 3 (cross-language probe).
- Type consistency: `resolve_heritage`, `HeritageNode`, `HeritageResolution`, `Snapshot::ambiguous_heritage`, `QueryPattern::InheritorsOf` use the same names in every task.
- Known risks, not tasks: ambiguous bases (e.g. two `Base` classes) lose impact precision versus CRG (explicit divergence); `AnalysisGraph` keeps ignoring text heritage, so flows/communities/risk are unchanged; the Go family has only embedding (`INHERITS` text), no structural satisfaction.
