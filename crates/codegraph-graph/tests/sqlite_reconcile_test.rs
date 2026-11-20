// ABOUTME: Verifies catalog reconciliation, readiness markers, FTS finalisation and the write lock.
// ABOUTME: The CRG compatibility probe runs only with CODEGRAPH_RUN_CRG_COMPAT=1.

use chrono::Utc;
use codegraph_core::{CodeNode, EdgeType, Language, Location, NodeType};
use codegraph_graph::sqlite_store::{SqliteGraphStore, WriteLock};
use codegraph_graph::{CodeEdge, FileMetadataRecord, ProjectMetadataRecord};
use rusqlite::Connection;
use std::path::Path;

fn node(id: u128, name: &str, file: &str, line: u32) -> CodeNode {
    let mut node = CodeNode::new(
        name,
        Some(NodeType::Function),
        Some(Language::Python),
        Location {
            file_path: file.to_string(),
            line,
            column: 0,
            end_line: Some(line + 1),
            end_column: None,
        },
    );
    node.id = uuid::Uuid::from_u128(id);
    node
}

fn edge(id: u128, from: &CodeNode, to: &CodeNode) -> CodeEdge {
    let mut edge = CodeEdge::new(from.id, to.id, EdgeType::Calls);
    edge.id = uuid::Uuid::from_u128(id);
    edge
}

fn file_meta(path: &str) -> FileMetadataRecord {
    FileMetadataRecord {
        file_path: path.to_string(),
        project_id: "p".into(),
        content_hash: format!("hash-{path}"),
        modified_at: Utc::now(),
        file_size: 1,
        last_indexed_at: Utc::now(),
        node_count: 1,
        edge_count: 0,
        language: Some("python".into()),
        parse_errors: None,
    }
}

fn marker() -> ProjectMetadataRecord {
    ProjectMetadataRecord {
        metadata: serde_json::json!({ "input_fingerprint": "fp-1" }),
        project_id: "p".into(),
        name: "fixture".into(),
        root_path: "/r".into(),
        primary_language: Some("python".into()),
        file_count: 2,
        node_count: 3,
        edge_count: 1,
        avg_coverage_score: 0.0,
        last_analyzed: Utc::now(),
        codegraph_version: "test".into(),
        organization_id: None,
        domain: None,
    }
}

fn conn(root: &Path) -> Connection {
    Connection::open(root.join(".code-review-graph/graph.db")).unwrap()
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// Graph with a.py:{main, Service} and b.py:{helper}, main → helper, main → Service.
async fn seeded(root: &Path) -> (SqliteGraphStore, Vec<CodeNode>) {
    let store = SqliteGraphStore::open(root).unwrap();
    let nodes = vec![
        node(1, "main", "a.py", 1),
        node(2, "Service", "a.py", 5),
        node(3, "helper", "b.py", 1),
    ];
    store.upsert_nodes_batch(&nodes).await.unwrap();
    store
        .upsert_edges_batch(&[
            edge(10, &nodes[0], &nodes[2]),
            edge(11, &nodes[0], &nodes[1]),
        ])
        .await
        .unwrap();
    store
        .upsert_file_metadata_batch(&[file_meta("a.py"), file_meta("b.py")])
        .await
        .unwrap();
    (store, nodes)
}

#[tokio::test]
async fn reconcile_removes_ids_outside_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let (store, nodes) = seeded(dir.path()).await;
    let keep = |n: &CodeNode| n.id.to_string();

    store
        .reconcile_catalog(
            "p",
            vec![keep(&nodes[0]), keep(&nodes[2])],
            vec![
                uuid::Uuid::from_u128(10).to_string(),
                uuid::Uuid::from_u128(11).to_string(),
            ],
            vec!["a.py".into()],
        )
        .await
        .unwrap();

    let c = conn(dir.path());
    assert_eq!(count(&c, "SELECT count(*) FROM nodes"), 2);
    assert_eq!(count(&c, "SELECT count(*) FROM cg_nodes"), 2);
    // Edge 11 points at the dropped Service node even though its id stayed in the catalog.
    assert_eq!(count(&c, "SELECT count(*) FROM edges"), 1);
    assert_eq!(count(&c, "SELECT count(*) FROM cg_edges"), 1);
    assert_eq!(count(&c, "SELECT count(*) FROM cg_file_metadata"), 1);
    let rows = store.get_file_metadata_for_project("p").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].file_path, "a.py");
    assert_eq!(rows[0].content_hash, "hash-a.py");
    assert!(store.get_file_metadata("b.py").await.unwrap().is_none());
    assert_eq!(
        store
            .get_file_metadata("a.py")
            .await
            .unwrap()
            .unwrap()
            .file_path,
        "a.py"
    );
}

#[tokio::test]
async fn counts_report_nodes_edges_and_files() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = seeded(dir.path()).await;
    let counts = store.counts().await.unwrap();
    assert_eq!((counts.nodes, counts.edges, counts.files), (3, 2, 2));
}

#[tokio::test]
async fn finalize_leaves_fts_searchable() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = seeded(dir.path()).await;
    store.finalize("full").await.unwrap();

    let c = conn(dir.path());
    let hit: String = c
        .query_row(
            "SELECT n.name FROM nodes_fts f JOIN nodes n ON n.id = f.rowid \
             WHERE nodes_fts MATCH 'service'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hit, "Service");
    assert_eq!(count(&c, "SELECT count(*) FROM nodes_fts_state"), 3);
    let meta = |key: &str| -> String {
        c.query_row("SELECT value FROM metadata WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .unwrap()
    };
    assert_eq!(meta("build_state"), "complete");
    assert_eq!(meta("last_build_type"), "full");
    assert_eq!(meta("fts_state_synced"), "2");
    assert_eq!(meta("repo_root"), dir.path().to_string_lossy());
    let updated = meta("last_updated");
    assert_eq!(updated.len(), "2026-11-19T21:21:39".len());
    assert!(!updated.contains('+') && !updated.ends_with('Z'));
}

#[tokio::test]
async fn finalize_resolves_pending_edges() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteGraphStore::open(dir.path()).unwrap();
    let (a, b) = (node(1, "a", "a.py", 1), node(2, "b", "a.py", 3));
    store.upsert_edges_batch(&[edge(10, &a, &b)]).await.unwrap();
    store.upsert_nodes_batch(&[a, b]).await.unwrap();
    store.finalize("incremental").await.unwrap();
    assert_eq!(
        count(
            &conn(dir.path()),
            "SELECT count(*) FROM edges WHERE source_qualified LIKE 'cg:%' \
             OR target_qualified LIKE 'cg:%'"
        ),
        0
    );
}

#[tokio::test]
async fn marker_round_trip_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteGraphStore::open(dir.path()).unwrap();
    assert!(store.project_marker("p").await.unwrap().is_none());
    store.upsert_project_metadata(marker()).await.unwrap();
    let stored = store.project_marker("p").await.unwrap().unwrap();
    assert_eq!(stored.metadata["input_fingerprint"], "fp-1");
    assert_eq!(stored.node_count, 3);
    store.delete_project_marker("p").await.unwrap();
    assert!(store.project_marker("p").await.unwrap().is_none());
}

#[test]
fn second_writer_lock_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let first = WriteLock::acquire(dir.path(), false)
        .unwrap()
        .expect("first lock");
    assert!(WriteLock::acquire(dir.path(), false).unwrap().is_none());

    let lock_file = dir.path().join(".code-review-graph/write.lock");
    let pid = std::fs::read_to_string(&lock_file).unwrap();
    assert_eq!(pid, format!("{}\n", std::process::id()));
    let external = std::process::Command::new("flock")
        .args(["-n", lock_file.to_str().unwrap(), "true"])
        .status()
        .unwrap();
    assert!(!external.success(), "flock -n must see the lock held");

    drop(first);
    assert_eq!(std::fs::read_to_string(&lock_file).unwrap(), "");
    assert!(WriteLock::acquire(dir.path(), false).unwrap().is_some());
}

/// Review Focus 1: code-review-graph opens a database we wrote without migrating it.
#[tokio::test]
async fn crg_reads_our_database() {
    if std::env::var("CODEGRAPH_RUN_CRG_COMPAT").as_deref() != Ok("1") {
        return;
    }
    let crg = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../code-review-graph");
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    let (store, _) = seeded(dir.path()).await;
    store.finalize("full").await.unwrap();
    drop(store);

    let out = std::process::Command::new("uv")
        .args(["run", "--project"])
        .arg(&crg)
        .args(["code-review-graph", "status", "--repo"])
        .arg(dir.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("Nodes: 3"), "{stdout}");
    assert!(stdout.contains("Edges: 2"), "{stdout}");
    let version: String = conn(dir.path())
        .query_row(
            "SELECT value FROM metadata WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(version, "15");
}
