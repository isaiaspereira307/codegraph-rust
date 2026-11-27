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

/// Snapshot dependency edges leaving `node` (INHERITS/IMPLEMENTS included).
async fn efferent(q: &GraphQuery, node: &str) -> i32 {
    q.calculate_coupling_metrics(node)
        .await
        .unwrap()
        .metrics
        .efferent_coupling
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
    assert_eq!(efferent(&q, "a.py::Child").await, 0);
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
    assert_eq!(efferent(&q, "t.py::MyTest").await, 0);
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
    let base = q
        .calculate_coupling_metrics("base.java::Base")
        .await
        .unwrap();
    assert_eq!(base.metrics.afferent_coupling, 1);
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

async fn inheritors_of(
    q: &GraphQuery,
    base: &str,
) -> codegraph_graph::graph_query::QueryGraphResult {
    use codegraph_graph::graph_query::QueryPattern;
    q.query_graph(QueryPattern::InheritorsOf, base, 0)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn inheritors_of_lists_subclasses_and_implementers() {
    let (_dir, q) = fixture(
        &[
            ("Interface", "a.ts::Shape", "a.ts", "typescript"),
            ("Class", "z.ts::Zed", "z.ts", "typescript"),
            ("Class", "b.ts::Box", "b.ts", "typescript"),
        ],
        &[
            ("INHERITS", "z.ts::Zed", "Shape"),
            ("IMPLEMENTS", "b.ts::Box", "Shape"),
        ],
    );
    let r = inheritors_of(&q, "a.ts::Shape").await;
    assert_eq!(r.pattern, "inheritors_of");
    let names: Vec<_> = r.results.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, vec!["Box", "Zed"]);
    assert_eq!(r.total, 2);
    assert_eq!(r.unresolved, 0);
}

#[tokio::test]
async fn inheritors_of_counts_ambiguous_bases_as_unresolved() {
    let (_dir, q) = fixture(
        &[
            ("Class", "a.py::Base", "a.py", "python"),
            ("Class", "b.py::Base", "b.py", "python"),
            ("Class", "c.py::Child", "c.py", "python"),
            ("Class", "d.py::Other", "d.py", "python"),
        ],
        &[
            ("INHERITS", "c.py::Child", "Base"),
            ("INHERITS", "d.py::Other", "Base"),
        ],
    );
    let r = inheritors_of(&q, "a.py::Base").await;
    assert!(r.results.is_empty());
    assert_eq!(r.unresolved, 2);
    // Another pattern keeps its own (SQL) count.
    use codegraph_graph::graph_query::QueryPattern;
    let c = q
        .query_graph(QueryPattern::CallersOf, "a.py::Base", 0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(c.unresolved, 0);
}

#[tokio::test]
async fn ambiguous_count_goes_only_to_the_candidates() {
    // A TS edge ambiguous between two TS bases says nothing about a Python `Base`.
    let (_dir, q) = fixture(
        &[
            ("Class", "a.py::Base", "a.py", "python"),
            ("Class", "x.ts::Base", "x.ts", "typescript"),
            ("Class", "y.ts::Base", "y.ts", "typescript"),
            ("Class", "z.ts::Child", "z.ts", "typescript"),
        ],
        &[("INHERITS", "z.ts::Child", "Base")],
    );
    assert_eq!(inheritors_of(&q, "a.py::Base").await.unresolved, 0);
    assert_eq!(inheritors_of(&q, "x.ts::Base").await.unresolved, 1);
    assert_eq!(inheritors_of(&q, "y.ts::Base").await.unresolved, 1);
}

#[tokio::test]
async fn inheritors_of_on_a_leaf_class_is_empty() {
    let (_dir, q) = fixture(
        &[
            ("Class", "a.py::Base", "a.py", "python"),
            ("Class", "c.py::Child", "c.py", "python"),
        ],
        &[("INHERITS", "c.py::Child", "Base")],
    );
    let r = inheritors_of(&q, "c.py::Child").await;
    assert!(r.results.is_empty());
    assert_eq!((r.total, r.unresolved), (0, 0));
}
