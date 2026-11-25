// ABOUTME: Verifies that text INHERITS/IMPLEMENTS targets bind to type nodes in the snapshot.
// ABOUTME: Rows are written as code-review-graph stores them: the base is the text the source wrote.

use codegraph_graph::graph_query::{GraphQuery, TraverseDirection};
use codegraph_graph::sqlite_store::SqliteGraphStore;
use rusqlite::Connection;
use tempfile::TempDir;

/// (kind, qualified_name, file_path, language); the name is the part after `::`.
type Node<'a> = (&'a str, &'a str, &'a str, &'a str);
/// (kind, source qualified name, target text).
type Edge<'a> = (&'a str, &'a str, &'a str);

fn fixture(nodes: &[Node], edges: &[Edge]) -> (TempDir, GraphQuery) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteGraphStore::open(dir.path()).unwrap();
    let conn = Connection::open(dir.path().join(".code-review-graph/graph.db")).unwrap();
    for (kind, qn, file, language) in nodes {
        let name = qn.rsplit("::").next().unwrap();
        conn.execute(
            "INSERT INTO nodes (kind, name, qualified_name, file_path, line_start, line_end, \
             language, is_test, extra, updated_at) VALUES (?1, ?2, ?3, ?4, 1, 10, ?5, 0, '{}', 0)",
            rusqlite::params![kind, name, qn, file, language],
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
    (dir, GraphQuery::new(store))
}

async fn inheritors(q: &GraphQuery, base: &str) -> Vec<(String, i32)> {
    q.traverse_graph(base, TraverseDirection::In, Some("extends"), 3, 100)
        .await
        .unwrap()
        .unwrap()
        .nodes
        .into_iter()
        .map(|n| (n.node.id, n.depth))
        .collect()
}

#[tokio::test]
async fn impact_radius_reaches_a_subclass_through_a_text_base() {
    let (_dir, q) = fixture(
        &[
            ("Class", "base.py::Base", "base.py", "python"),
            ("Class", "child.py::Child", "child.py", "python"),
        ],
        &[("INHERITS", "child.py::Child", "base.Base")],
    );
    let impact = q
        .get_impact_radius(&["base.py::Base".to_string()], &[], 2, 100)
        .await
        .unwrap();
    let got: Vec<_> = impact
        .impacted
        .iter()
        .map(|n| (n.node.id.as_str(), n.depth))
        .collect();
    assert_eq!(got, vec![("child.py::Child", 1)]);
}

#[tokio::test]
async fn traverse_graph_follows_extends_in_reverse() {
    let (_dir, q) = fixture(
        &[
            ("Class", "a.ts::Animal", "a.ts", "typescript"),
            ("Class", "d.ts::Dog", "d.ts", "typescript"),
            ("Class", "p.js::Puppy", "p.js", "javascript"),
        ],
        &[
            ("INHERITS", "d.ts::Dog", "Animal"),
            ("INHERITS", "p.js::Puppy", "Dog"),
        ],
    );
    assert_eq!(
        inheritors(&q, "a.ts::Animal").await,
        vec![("d.ts::Dog".to_string(), 1), ("p.js::Puppy".to_string(), 2)]
    );
}

#[tokio::test]
async fn ambiguous_base_adds_no_edge() {
    let (_dir, q) = fixture(
        &[
            ("Class", "b.py::Base", "b.py", "python"),
            ("Class", "c.py::Base", "c.py", "python"),
            ("Class", "a.py::Child", "a.py", "python"),
        ],
        &[("INHERITS", "a.py::Child", "Base")],
    );
    assert!(inheritors(&q, "b.py::Base").await.is_empty());
    assert!(inheritors(&q, "c.py::Base").await.is_empty());
    assert!(q.get_hub_nodes(1).await.unwrap().is_empty());
}

#[tokio::test]
async fn external_base_adds_no_edge() {
    let (_dir, q) = fixture(
        &[
            ("Class", "t.py::MyTest", "t.py", "python"),
            ("Function", "t.py::TestCase", "t.py", "python"),
        ],
        &[
            ("INHERITS", "t.py::MyTest", "unittest.TestCase"),
            ("INHERITS", "t.py::MyTest", "object"),
        ],
    );
    let out = q
        .traverse_graph("t.py::MyTest", TraverseDirection::Out, None, 3, 100)
        .await
        .unwrap()
        .unwrap();
    assert!(out.nodes.is_empty());
    assert!(q.get_hub_nodes(1).await.unwrap().is_empty());
}

#[tokio::test]
async fn existing_qualified_target_is_not_duplicated() {
    let (_dir, q) = fixture(
        &[
            ("Class", "base.java::Base", "base.java", "java"),
            ("Class", "child.java::Child", "child.java", "java"),
        ],
        &[("INHERITS", "child.java::Child", "base.java::Base")],
    );
    let hubs = q.get_hub_nodes(1).await.unwrap();
    let base = hubs
        .iter()
        .find(|h| h.node_id == "base.java::Base")
        .unwrap();
    assert_eq!(base.afferent_degree, 1);
    assert_eq!(
        inheritors(&q, "base.java::Base").await,
        vec![("child.java::Child".to_string(), 1)]
    );
}

#[tokio::test]
async fn imported_file_base_wins_over_unrelated_duplicate_through_the_snapshot() {
    let (_dir, q) = fixture(
        &[
            ("File", "app.py", "app.py", "python"),
            ("File", "models.py", "models.py", "python"),
            ("File", "legacy.py", "legacy.py", "python"),
            ("Class", "app.py::Child", "app.py", "python"),
            ("Class", "models.py::Base", "models.py", "python"),
            ("Class", "legacy.py::Base", "legacy.py", "python"),
        ],
        &[
            ("IMPORTS_FROM", "app.py", "models.py"),
            ("INHERITS", "app.py::Child", "models.Base"),
        ],
    );
    assert_eq!(
        inheritors(&q, "models.py::Base").await,
        vec![("app.py::Child".to_string(), 1)]
    );
    assert!(inheritors(&q, "legacy.py::Base").await.is_empty());
}
