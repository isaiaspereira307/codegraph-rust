# SQLite storage (code-review-graph v15 schema)

CodeGraph stores each repository's graph in `<repo>/.code-review-graph/graph.db`, the same
file and schema that code-review-graph (CRG) 2.4 uses. Either tool can read a database the
other wrote. Code: `crates/codegraph-graph/src/sqlite_store/`.

## Schema

- `schema_v15.sql` is the DDL copied verbatim from a database CRG created (`sqlite_master`
  text must match, `fresh_schema_matches_crg_created_schema` checks this). It is applied only
  to a new file, and `metadata.schema_version` is set to `15`.
- An existing database must have `schema_version = 15`. Any other value is refused with
  "run code-review-graph update"; CodeGraph never runs migrations.
- `schema_cg.sql` adds Rust-only tables that CRG ignores:

| Table | Holds |
|---|---|
| `cg_nodes` | stable SHA node id (`uuid`) → `qualified_name`, Rust kind, complexity, source text, attributes |
| `cg_edges` | stable SHA edge id → `edges.id`, endpoint uuids |
| `cg_file_metadata` | `FileMetadataRecord` JSON per `(project_id, file_path)` |
| `cg_project_metadata` | `ProjectMetadataRecord` JSON (the "index ready" marker) |

The database is opened in WAL mode with a 5 s busy timeout. Writers take the same advisory
`flock` CRG uses, `.code-review-graph/write.lock`.

## Identity

Paths are stored absolute. A file node's `qualified_name` is its path; any other node's is
`<path>::<symbol>`, where the symbol is the node's `qualified_name` attribute (without the
path prefix) or its name. When two distinct Rust nodes produce the same `qualified_name`,
the later one gets the suffix `@L<line>`.

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
