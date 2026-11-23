// ABOUTME: Builds a SQLite graph.db fixture by inserting rows straight into nodes/edges.
// ABOUTME: No cg_nodes rows, so it mimics a database written by code-review-graph.

#![allow(dead_code)]

use codegraph_graph::sqlite_store::SqliteGraphStore;
use rusqlite::Connection;
use tempfile::TempDir;

pub type NodeRow<'a> = (&'a str, &'a str, &'a str, &'a str, bool);
pub type EdgeRow<'a> = (&'a str, &'a str, &'a str);

/// `nodes`: (kind, qualified_name, name, file_path, is_test). `edges`: (kind, source, target).
pub fn fixture_db(nodes: &[NodeRow], edges: &[EdgeRow]) -> (TempDir, SqliteGraphStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteGraphStore::open(dir.path()).unwrap();
    let conn = open_conn(dir.path());
    for (kind, qn, name, file, is_test) in nodes {
        conn.execute(
            "INSERT INTO nodes (kind, name, qualified_name, file_path, line_start, line_end, \
             is_test, extra, updated_at) VALUES (?1, ?2, ?3, ?4, 1, 10, ?5, '{}', 0)",
            rusqlite::params![kind, name, qn, file, *is_test as i64],
        )
        .unwrap();
    }
    for (kind, src, tgt) in edges {
        conn.execute(
            "INSERT INTO edges (kind, source_qualified, target_qualified, file_path, line, \
             updated_at) VALUES (?1, ?2, ?3, '', 1, 0)",
            rusqlite::params![kind, src, tgt],
        )
        .unwrap();
    }
    (dir, store)
}

pub fn open_conn(root: &std::path::Path) -> Connection {
    Connection::open(root.join(".code-review-graph/graph.db")).unwrap()
}
