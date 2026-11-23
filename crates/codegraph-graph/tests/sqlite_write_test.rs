// ABOUTME: Verifies CodeNode/CodeEdge writes land in the CRG nodes/edges tables.
// ABOUTME: Covers identity, duplicate symbols, edge ordering, idempotence and file deletion.

use chrono::Utc;
use codegraph_core::{CodeNode, EdgeType, Language, Location, NodeType};
use codegraph_graph::sqlite_store::SqliteGraphStore;
use codegraph_graph::sqlite_store::mapping::{crg_edge_kind, crg_node_kind, edge_kind_from_input};
use codegraph_graph::{CodeEdge, FileMetadataRecord};
use rusqlite::Connection;
use std::path::Path;

fn node(id: u128, name: &str, kind: NodeType, file: &str, line: u32) -> CodeNode {
    let mut node = CodeNode::new(
        name,
        Some(kind),
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

fn edge(id: u128, from: &CodeNode, to: &CodeNode, kind: EdgeType) -> CodeEdge {
    let mut edge = CodeEdge::new(from.id, to.id, kind);
    edge.id = uuid::Uuid::from_u128(id);
    edge
}

fn file_meta(path: &str) -> FileMetadataRecord {
    FileMetadataRecord {
        file_path: path.to_string(),
        project_id: "p".into(),
        content_hash: "h".into(),
        modified_at: Utc::now(),
        file_size: 1,
        last_indexed_at: Utc::now(),
        node_count: 1,
        edge_count: 0,
        language: Some("python".into()),
        parse_errors: None,
    }
}

fn open(root: &Path) -> (SqliteGraphStore, Connection) {
    let store = SqliteGraphStore::open(root).unwrap();
    let conn = Connection::open(root.join(".code-review-graph/graph.db")).unwrap();
    (store, conn)
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn abs(root: &Path, rel: &str) -> String {
    root.join(rel).to_string_lossy().into_owned()
}

#[tokio::test]
async fn node_round_trip_uses_crg_columns() {
    let dir = tempfile::tempdir().unwrap();
    let (store, conn) = open(dir.path());
    let mut run = node(1, "run", NodeType::Function, "pkg/svc.py", 5)
        .with_content("def run(self, x): ...")
        .with_complexity(3.0);
    run.metadata
        .attributes
        .insert("qualified_name".into(), "Service.run".into());
    let class = node(2, "Service", NodeType::Struct, "pkg/svc.py", 4);
    store.upsert_nodes_batch(&[run, class]).await.unwrap();

    let file = abs(dir.path(), "pkg/svc.py");
    let row: (
        String,
        String,
        String,
        String,
        i64,
        i64,
        String,
        String,
        i64,
        String,
        String,
    ) = conn
        .query_row(
            "SELECT kind, name, qualified_name, file_path, line_start, line_end, language, \
             symbol, is_test, extra, name_tokens FROM nodes WHERE name = 'run'",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        row,
        (
            "Function".into(),
            "run".into(),
            format!("{file}::Service.run"),
            file.clone(),
            5,
            6,
            "python".into(),
            "Service.run".into(),
            0,
            "{}".into(),
            String::new(),
        )
    );
    let (uuid, rust_kind, complexity, content): (String, String, f64, String) = conn
        .query_row(
            "SELECT uuid, rust_kind, complexity, content FROM cg_nodes WHERE qualified_name = ?1",
            [format!("{file}::Service.run")],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(uuid, uuid::Uuid::from_u128(1).to_string());
    assert_eq!(rust_kind, "Function");
    assert_eq!(complexity, 3.0);
    assert_eq!(content, "def run(self, x): ...");
    let class_kind: String = conn
        .query_row("SELECT kind FROM nodes WHERE name = 'Service'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(class_kind, "Class");
    assert!(count(&conn, "SELECT count(*) FROM nodes WHERE updated_at > 0") == 2);
}

#[tokio::test]
async fn name_tokens_follow_crg_camel_case_split() {
    let dir = tempfile::tempdir().unwrap();
    let (store, conn) = open(dir.path());
    let mut n = node(1, "hashPassword", NodeType::Function, "authUtils.py", 1);
    n.metadata
        .attributes
        .insert("parent_name".into(), "UserService".into());
    store.upsert_nodes_batch(&[n]).await.unwrap();
    let tokens: String = conn
        .query_row("SELECT name_tokens FROM nodes", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tokens, "hash Password User Service auth Utils");
}

#[tokio::test]
async fn file_node_is_named_and_qualified_by_absolute_path() {
    let dir = tempfile::tempdir().unwrap();
    let (store, conn) = open(dir.path());
    let mut file = node(
        1,
        "./pkg/svc.py",
        NodeType::Other("File".into()),
        "./pkg/svc.py",
        1,
    );
    file.metadata
        .attributes
        .insert("qualified_name".into(), "./pkg/svc.py".into());
    let mut run = node(2, "run", NodeType::Function, "./pkg/svc.py", 3);
    run.metadata
        .attributes
        .insert("qualified_name".into(), "./pkg/svc.py::Svc.run".into());
    run.metadata
        .attributes
        .insert("parent_name".into(), "Svc".into());
    store.upsert_nodes_batch(&[file, run]).await.unwrap();

    let path = abs(dir.path(), "pkg/svc.py");
    let row: (String, String, String) = conn
        .query_row(
            "SELECT kind, name, qualified_name FROM nodes WHERE kind = 'File'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(row, ("File".into(), path.clone(), path.clone()));
    let run: (String, String) = conn
        .query_row(
            "SELECT qualified_name, parent_name FROM nodes WHERE name = 'run'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(run, (format!("{path}::Svc.run"), "Svc".into()));
}

#[tokio::test]
async fn duplicate_symbol_gets_line_suffix() {
    let dir = tempfile::tempdir().unwrap();
    let (store, conn) = open(dir.path());
    let first = node(1, "os", NodeType::Import, "a.py", 1);
    let second = node(2, "os", NodeType::Import, "a.py", 3);
    store
        .upsert_nodes_batch(&[first.clone(), second])
        .await
        .unwrap();
    store.upsert_nodes_batch(&[first]).await.unwrap();

    let file = abs(dir.path(), "a.py");
    let mut stmt = conn
        .prepare("SELECT qualified_name FROM nodes ORDER BY line_start")
        .unwrap();
    let names: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(names, vec![format!("{file}::os"), format!("{file}::os@L3")]);
    assert_eq!(count(&conn, "SELECT count(*) FROM cg_nodes"), 2);
}

#[tokio::test]
async fn edge_before_nodes_is_resolved_on_flush() {
    let dir = tempfile::tempdir().unwrap();
    let (store, conn) = open(dir.path());
    let caller = node(1, "main", NodeType::Function, "a.py", 1);
    let callee = node(2, "helper", NodeType::Function, "b.py", 1);
    store
        .upsert_edges_batch(&[edge(10, &caller, &callee, EdgeType::Calls)])
        .await
        .unwrap();
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM edges WHERE source_qualified LIKE 'cg:%'"
        ),
        1
    );
    store.upsert_nodes_batch(&[caller, callee]).await.unwrap();
    assert_eq!(store.resolve_pending_edges().await.unwrap(), 1);

    let (kind, source, target, file): (String, String, String, String) = conn
        .query_row(
            "SELECT kind, source_qualified, target_qualified, file_path FROM edges",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(kind, "CALLS");
    assert_eq!(source, format!("{}::main", abs(dir.path(), "a.py")));
    assert_eq!(target, format!("{}::helper", abs(dir.path(), "b.py")));
    assert_eq!(file, abs(dir.path(), "a.py"));
}

#[tokio::test]
async fn edge_upsert_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let (store, conn) = open(dir.path());
    let a = node(1, "a", NodeType::Function, "a.py", 1);
    let b = node(2, "b", NodeType::Function, "a.py", 5);
    store
        .upsert_nodes_batch(&[a.clone(), b.clone()])
        .await
        .unwrap();
    let e = edge(10, &a, &b, EdgeType::Calls);
    store.upsert_edges_batch(&[e.clone()]).await.unwrap();
    store.upsert_edges_batch(&[e]).await.unwrap();
    assert_eq!(count(&conn, "SELECT count(*) FROM edges"), 1);
    assert_eq!(count(&conn, "SELECT count(*) FROM cg_edges"), 1);
}

#[tokio::test]
async fn delete_files_removes_nodes_edges_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let (store, conn) = open(dir.path());
    let a = node(1, "a", NodeType::Function, "f1.py", 1);
    let b = node(2, "b", NodeType::Function, "f2.py", 1);
    store
        .upsert_nodes_batch(&[a.clone(), b.clone()])
        .await
        .unwrap();
    store
        .upsert_edges_batch(&[edge(10, &b, &a, EdgeType::Calls)])
        .await
        .unwrap();
    store
        .upsert_file_metadata_batch(&[file_meta("f1.py"), file_meta("f2.py")])
        .await
        .unwrap();

    store
        .delete_data_for_files("p", &["f1.py".to_string()])
        .await
        .unwrap();

    assert_eq!(count(&conn, "SELECT count(*) FROM nodes"), 1);
    assert_eq!(count(&conn, "SELECT count(*) FROM cg_nodes"), 1);
    assert_eq!(count(&conn, "SELECT count(*) FROM edges"), 0);
    assert_eq!(count(&conn, "SELECT count(*) FROM cg_edges"), 0);
    let remaining: String = conn
        .query_row("SELECT file_path FROM cg_file_metadata", [], |r| r.get(0))
        .unwrap();
    assert_eq!(remaining, "f2.py");
}

#[test]
fn edge_kind_mapping_round_trips() {
    let cases = [
        (EdgeType::Calls, "CALLS"),
        (EdgeType::Imports, "IMPORTS_FROM"),
        (EdgeType::Extends, "INHERITS"),
        (EdgeType::Implements, "IMPLEMENTS"),
        (EdgeType::Contains, "CONTAINS"),
        (EdgeType::References, "REFERENCES"),
        (EdgeType::Defines, "DEFINES"),
        (EdgeType::Uses, "USES"),
        (EdgeType::Other("flows_to".into()), "FLOWS_TO"),
    ];
    for (edge_type, kind) in cases {
        assert_eq!(crg_edge_kind(&edge_type), kind);
        assert_eq!(edge_kind_from_input(&edge_type.to_string()), kind);
        assert_eq!(edge_kind_from_input(kind), kind);
    }
    assert_eq!(edge_kind_from_input("imports"), "IMPORTS_FROM");
    assert_eq!(crg_node_kind(&NodeType::Trait), "Class");
    assert_eq!(crg_node_kind(&NodeType::Enum), "Type");
    assert_eq!(crg_node_kind(&NodeType::Module), "Module");
}

#[tokio::test]
async fn unresolved_call_is_kept_as_text() {
    let dir = tempfile::tempdir().unwrap();
    let (store, conn) = open(dir.path());
    let f = node(1, "f", NodeType::Function, "a.py", 1);
    store
        .upsert_nodes_batch(std::slice::from_ref(&f))
        .await
        .unwrap();
    let mut e = CodeEdge::unresolved(f.id, "print", EdgeType::Calls)
        .with_metadata("line".into(), "2".into());
    e.set_deterministic_id("p");
    assert_eq!(e.unresolved_target(), Some("print"));
    store.upsert_edges_batch(&[e.clone()]).await.unwrap();
    store.upsert_edges_batch(&[e.clone()]).await.unwrap();
    store
        .reconcile_catalog(
            "p",
            vec![f.id.to_string()],
            vec![e.id.to_string()],
            vec!["a.py".into()],
        )
        .await
        .unwrap();
    store.finalize("full").await.unwrap();

    let row: (String, String, String, i64, Option<String>) = conn
        .query_row(
            "SELECT source_qualified, target_qualified, file_path, line, target_resolution \
             FROM edges WHERE kind = 'CALLS'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (
            format!("{}::f", abs(dir.path(), "a.py")),
            "print".to_string(),
            abs(dir.path(), "a.py"),
            2,
            Some("unresolved".to_string())
        )
    );
    assert_eq!(count(&conn, "SELECT count(*) FROM edges"), 1);
    assert_eq!(count(&conn, "SELECT count(*) FROM cg_edges"), 1);

    // An unresolved import keeps the module text and no call resolution.
    let file = node(2, "a.py", NodeType::Other("File".into()), "a.py", 1);
    store
        .upsert_nodes_batch(std::slice::from_ref(&file))
        .await
        .unwrap();
    let mut imp = CodeEdge::unresolved(file.id, "os", EdgeType::Imports);
    imp.set_deterministic_id("p");
    store.upsert_edges_batch(&[imp]).await.unwrap();
    let (target, resolution): (String, Option<String>) = conn
        .query_row(
            "SELECT target_qualified, target_resolution FROM edges WHERE kind = 'IMPORTS_FROM'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((target.as_str(), resolution), ("os", None));
}

#[tokio::test]
async fn tested_by_mirrors_calls_from_tests() {
    let dir = tempfile::tempdir().unwrap();
    let (store, conn) = open(dir.path());
    let f = node(1, "f", NodeType::Function, "a.py", 1);
    let t = node(2, "test_x", NodeType::Other("Test".into()), "test_a.py", 1);
    let g = node(3, "g", NodeType::Function, "a.py", 5);
    store
        .upsert_nodes_batch(&[f.clone(), t.clone(), g.clone()])
        .await
        .unwrap();
    let call_f = edge(10, &t, &f, EdgeType::Calls).with_metadata("line".into(), "2".into());
    let mut call_assert = CodeEdge::unresolved(t.id, "assert", EdgeType::Calls)
        .with_metadata("line".into(), "3".into());
    call_assert.id = uuid::Uuid::from_u128(11);
    // A non-test caller produces no TESTED_BY.
    let call_g = edge(12, &f, &g, EdgeType::Calls).with_metadata("line".into(), "2".into());
    store
        .upsert_edges_batch(&[call_f, call_assert, call_g])
        .await
        .unwrap();
    let tested_by = |conn: &Connection| -> Vec<(String, String, String, i64)> {
        let mut stmt = conn
            .prepare(
                "SELECT source_qualified, target_qualified, file_path, line FROM edges \
                 WHERE kind = 'TESTED_BY' ORDER BY line",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    store.finalize("full").await.unwrap();
    // Reconciliation keeps them until finalize rebuilds them.
    store
        .reconcile_catalog(
            "p",
            [&f, &t, &g].iter().map(|n| n.id.to_string()).collect(),
            [10u128, 11, 12]
                .iter()
                .map(|i| uuid::Uuid::from_u128(*i).to_string())
                .collect(),
            vec!["a.py".into(), "test_a.py".into()],
        )
        .await
        .unwrap();
    assert_eq!(
        count(&conn, "SELECT count(*) FROM edges WHERE kind = 'TESTED_BY'"),
        2
    );
    // Finalising again (incremental runs) replaces rather than duplicates.
    store.finalize("incremental").await.unwrap();
    let test_qn = format!("{}::test_x", abs(dir.path(), "test_a.py"));
    let test_file = abs(dir.path(), "test_a.py");
    assert_eq!(
        tested_by(&conn),
        vec![
            (
                format!("{}::f", abs(dir.path(), "a.py")),
                test_qn.clone(),
                test_file.clone(),
                2
            ),
            ("assert".to_string(), test_qn, test_file, 3),
        ]
    );
}

#[tokio::test]
async fn writer_stores_decorators_in_extra() {
    let dir = tempfile::tempdir().unwrap();
    let (store, conn) = open(dir.path());
    let mut decorated = node(1, "login", NodeType::Function, "app.py", 3);
    decorated
        .metadata
        .attributes
        .insert("decorators".into(), "app.get, login_required,".into());
    let plain = node(2, "plain", NodeType::Function, "app.py", 9);
    store.upsert_nodes_batch(&[decorated, plain]).await.unwrap();
    let extra = |name: &str| -> String {
        conn.query_row("SELECT extra FROM nodes WHERE name = ?1", [name], |r| {
            r.get(0)
        })
        .unwrap()
    };
    assert_eq!(
        extra("login"),
        r#"{"decorators":["app.get","login_required"]}"#
    );
    assert_eq!(extra("plain"), "{}");
}
