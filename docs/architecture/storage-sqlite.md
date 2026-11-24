# SQLite storage (code-review-graph v15 schema)

CodeGraph stores each repository's graph in `<repo>/.code-review-graph/graph.db`, the same
file and schema that code-review-graph (CRG) 2.4 uses. Either tool can read a database the
other wrote. Code: `crates/codegraph-graph/src/sqlite_store/`.

## Schema

- `schema_v15.sql` is the DDL copied verbatim from a database CRG created (`sqlite_master`
  text must match, `fresh_schema_matches_crg_created_schema` checks this). It is applied only
  to a new file, and `metadata.schema_version` is set to `15`.
- An existing database must have `schema_version = 15`. Any other value is refused with an
  error that names the database path, says whether an older or newer CRG wrote it, and gives
  the fix (upgrade CRG and `code-review-graph update`, or delete the file and run
  `codegraph index`). CodeGraph never runs migrations.

## Coexisting with an installed code-review-graph

CRG 2.3.x writes schema 9, which CodeGraph refuses. CRG's Claude Code hooks (`status` on
SessionStart, `update` after Edit/Write) rewrite the database on every session, so deleting it
once is not enough. Two ways out:

1. Upgrade CRG to 2.4.0+ (schema 15). Both tools then share the file. Each `update` rewrites
   the nodes with CRG's extractor, so pick one writer per repository.
2. Keep CRG 2.3.x and make its hooks skip repositories that CodeGraph owns. CRG has no
   per-repository switch, so the guard lives in the hook command; the convention is a marker
   file `.code-review-graph/crg-hooks-off` (ignored by the directory's own `.gitignore`):

   ```sh
   root="$(git rev-parse --show-toplevel 2>/dev/null)" || exit 0
   [ -e "$root/.code-review-graph/crg-hooks-off" ] && exit 0
   code-review-graph update --skip-flows --repo "$root"
   ```

   Then, in the repository: `rm .code-review-graph/graph.db*`,
   `touch .code-review-graph/crg-hooks-off`, `codegraph index . -r`. Also leave CRG's MCP
   server disabled there (`disabledMcpjsonServers` in `.claude/settings.local.json`).
- `schema_cg.sql` adds Rust-only tables that CRG ignores:

| Table | Holds |
|---|---|
| `cg_nodes` | stable SHA node id (`uuid`) → `qualified_name`, Rust kind, complexity, source text, attributes |
| `cg_edges` | stable SHA edge id → `edges.id`, endpoint uuids |
| `cg_file_metadata` | `FileMetadataRecord` JSON per `(project_id, file_path)` |
| `cg_project_metadata` | `ProjectMetadataRecord` JSON (the "index ready" marker) |

## Derived analysis: flows and communities

At the end of `finalize`, after the FTS refresh, a post-process recomputes the CRG analysis
tables in one write transaction (`crates/codegraph-graph/src/analysis/`). `finalize` runs on
every `index`, `build`, `update` and watch re-index that rebuilds the graph. A run over
unchanged sources skips the build and `finalize`, except when the requested level is `full`
and `metadata.postprocess_level` (the level of the last post-process, written by `finalize`)
is not `full`: a graph built with `--skip-flows`/`--skip-postprocess`, written by an older
build, or whose last analysis failed then gets `finalize` (and its post-process) on that run.

| Table / column | Written |
|---|---|
| `flows` | one row per execution flow traced from a CRG-style entry point (`name`, `entry_point_id`, `depth`, `node_count`, `file_count`, `criticality`, `path_json` = node ids) |
| `flow_memberships` | `(flow_id, node_id, position)` for every node in a flow |
| `communities` | Leiden communities (deterministic: no RNG, greedy refinement) with CRG names, `cohesion`, `size`, `dominant_language`; directory-based fallback (detector `file`) when the graph has no edges |
| `nodes.community_id` | each node's community |
| `metadata.community_detector` | `leiden` or `file`, as CRG records it |

Both are rebuilt from scratch each time (no incremental flows; `flows.id` changes per build;
the `communities` AUTOINCREMENT restarts, so an unchanged graph keeps the same community ids). The level comes
from `--skip-flows` (`minimal`) / `--skip-postprocess` (`none`) on `index`, `build` and
`update`, else `CODEGRAPH_POSTPROCESS` (`full` by default; anything else is an error).
`minimal` and `none` skip the step and leave the previous rows in place (they coincide: CRG's
`minimal` adds resolvers, `nodes.signature` and FTS, and the Rust `finalize` always resolves
and rebuilds FTS but never computes signatures). Unlike CRG's `none`, `none` here still
resolves pending edges and rebuilds FTS and TESTED_BY in `finalize` (harmless). The invalid-level
error is reported when the indexer starts, before any file is parsed. A failing post-process
does not fail the build: like CRG, it logs a warning, records `postprocess_level = failed`
and the error in `metadata.postprocess_warning` (removed by the next successful run), and
the build's ready marker is still written. The flow, community,
architecture, hub/bridge, knowledge-gap, surprising-connection and suggested-question tools and
the review `risk_score` read these tables; CRG-written rows are read the same way.

## Concurrency

The database is opened in WAL mode with a 5 s busy timeout. Writers take the same advisory
`flock` CRG uses, `.code-review-graph/write.lock`.

## Identity

Paths are stored absolute. A file node's `qualified_name` is its path; any other node's is
`<path>::<symbol>`, where the symbol is the node's `qualified_name` attribute (without the
path prefix) or its name. When two distinct Rust nodes produce the same `qualified_name`,
the later one gets the suffix `@L<line>`.

Node IDs (`cg_nodes.uuid`) hash the project id, the file key, the name, the kind and the start
line. The file key is the path relative to the project root (`canonical_file_key`), so indexing
with `.`, a relative path or an absolute path gives the same IDs. Only the ID uses the key:
`qualified_name` keeps the path as the indexer received it, so a spelling with `..` is stored
un-normalized in that column (open item).

Edges from `CALLS` and `IMPORTS_FROM` (and `TESTED_BY`, derived from them) carry the 1-based
line of the call or import in `line`, computed from the extractor's byte span.

Tools report a node by its `cg_nodes.uuid`; nodes CRG wrote (no uuid) are reported by
`qualified_name`. Both forms are accepted as input.

## Kind mapping

| Rust `NodeType` | `nodes.kind` |
|---|---|
| Function | Function |
| Class, Struct, Trait, Interface | Class |
| Enum, Type | Type |
| File | File (one per source file, `qualified_name` = path) |
| Test | Test (`is_test = 1`; test functions/methods) |
| Module, Directory, Variable, Other(s) | kept as the Rust name (no CRG equivalent) |

`Import` nodes are no longer emitted: an import is an `IMPORTS_FROM` edge from the file to the
imported module, resolved per language to a file path like CRG and otherwise kept as text.

| Rust `EdgeType` | `edges.kind` |
|---|---|
| Calls | CALLS |
| Imports | IMPORTS_FROM |
| Extends | INHERITS |
| Implements | IMPLEMENTS |
| Contains | CONTAINS (File → top-level symbol, class → method) |
| References | REFERENCES |
| Defines, Uses, Other(s) | upper-cased name (`DEFINES`, `USES`, …) |

The original Rust kind is kept in `cg_nodes.rust_kind`. Produced like CRG: `File` and `Test`
nodes, `CONTAINS`, `IMPORTS_FROM` to a file path, and `TESTED_BY` (derived when indexing
finishes from `Calls` edges whose source is a `Test` node). Call and import targets that do
not resolve are stored as text (the unresolved name) instead of being dropped. Remaining
losses: `Test` detection needs decorators for Rust/Java (not recorded yet), and CRG-only edge
kinds such as `HANDLES` are visible to the tools under their CRG names.
