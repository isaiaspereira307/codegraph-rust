// ABOUTME: Verifies the SQLite store creates and opens code-review-graph v15 databases.
// ABOUTME: Covers fresh creation, databases built by CRG itself and unsupported versions.

use codegraph_graph::sqlite_store::{SqliteGraphStore, db_path};
use rusqlite::Connection;
use std::path::Path;

const CRG_TABLES: &[&str] = &[
    "nodes",
    "edges",
    "metadata",
    "flows",
    "flow_memberships",
    "communities",
    "nodes_fts_state",
    "nodes_fts",
];
const CRG_INDEXES: &[&str] = &[
    "idx_nodes_file",
    "idx_nodes_kind",
    "idx_nodes_name",
    "idx_edges_kind",
    "idx_edges_target_kind",
    "idx_edges_source_kind",
    "idx_edges_file",
    "idx_flows_criticality",
    "idx_flows_entry",
    "idx_flow_memberships_node",
    "idx_nodes_community",
    "idx_communities_parent",
    "idx_communities_cohesion",
    "idx_edges_composite",
    "idx_nodes_symbol",
    "idx_edges_kind_target_resolution",
    "idx_nodes_fts_state_file",
];
const CG_TABLES: &[&str] = &[
    "cg_nodes",
    "cg_edges",
    "cg_file_metadata",
    "cg_project_metadata",
];

fn names(conn: &Connection, kind: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = ?1 ORDER BY name")
        .unwrap();
    stmt.query_map([kind], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn crg_table_sql(conn: &Connection) -> Vec<(String, String)> {
    let mut stmt = conn
        .prepare(
            "SELECT name, sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'cg_%' \
             AND name NOT LIKE 'idx_cg_%' ORDER BY name",
        )
        .unwrap();
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn install_fixture(root: &Path) {
    let path = db_path(root);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/crg_v15_empty.db"),
        &path,
    )
    .unwrap();
}

#[test]
fn fresh_database_has_crg_v15_layout() {
    let dir = tempfile::tempdir().unwrap();
    let _store = SqliteGraphStore::open(dir.path()).unwrap();
    let path = dir.path().join(".code-review-graph/graph.db");
    assert_eq!(db_path(dir.path()), path);
    assert!(path.exists());

    let conn = Connection::open(&path).unwrap();
    let tables = names(&conn, "table");
    for table in CRG_TABLES.iter().chain(CG_TABLES) {
        assert!(tables.contains(&table.to_string()), "missing table {table}");
    }
    for table in ["community_summaries", "flow_snapshots", "risk_index"] {
        assert!(!tables.contains(&table.to_string()), "v15 dropped {table}");
    }
    let indexes = names(&conn, "index");
    for index in CRG_INDEXES {
        assert!(
            indexes.contains(&index.to_string()),
            "missing index {index}"
        );
    }
    let version: String = conn
        .query_row(
            "SELECT value FROM metadata WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(version, "15");
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}

#[test]
fn fresh_schema_matches_crg_created_schema() {
    let fresh = tempfile::tempdir().unwrap();
    SqliteGraphStore::open(fresh.path()).unwrap();
    let ours = crg_table_sql(&Connection::open(db_path(fresh.path())).unwrap());
    let crg = crg_table_sql(
        &Connection::open(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/crg_v15_empty.db"),
        )
        .unwrap(),
    );
    assert_eq!(ours, crg);
}

#[test]
fn opens_database_created_by_crg() {
    let dir = tempfile::tempdir().unwrap();
    install_fixture(dir.path());
    let before = crg_table_sql(&Connection::open(db_path(dir.path())).unwrap());

    let _store = SqliteGraphStore::open(dir.path()).unwrap();

    let conn = Connection::open(db_path(dir.path())).unwrap();
    assert_eq!(crg_table_sql(&conn), before);
    let tables = names(&conn, "table");
    for table in CG_TABLES {
        assert!(tables.contains(&table.to_string()), "missing table {table}");
    }
}

#[test]
fn rejects_other_schema_versions() {
    for version in ["14", "16"] {
        let dir = tempfile::tempdir().unwrap();
        install_fixture(dir.path());
        Connection::open(db_path(dir.path()))
            .unwrap()
            .execute(
                "UPDATE metadata SET value = ?1 WHERE key = 'schema_version'",
                [version],
            )
            .unwrap();
        let err = SqliteGraphStore::open(dir.path())
            .err()
            .expect("must refuse");
        assert_eq!(
            err.to_string(),
            format!(
                "graph.db schema version {version} is not supported (expected 15); \
                 run code-review-graph update"
            )
        );
    }
}
