// ABOUTME: Tests flow entry-point detection over hand-built SQLite graphs.
// ABOUTME: Covers root detection, framework decorators, name patterns and language rules.

#[path = "common/analysis_fixture.rs"]
mod analysis_fixture;

use analysis_fixture::{NodeRow, fixture_db, open_conn};
use codegraph_graph::analysis::AnalysisGraph;
use codegraph_graph::analysis::flows::detect_entry_points;

fn qnames(g: &AnalysisGraph, idx: &[usize]) -> Vec<String> {
    idx.iter().map(|&i| g.nodes[i].qualified.clone()).collect()
}

fn graph(
    nodes: &[NodeRow],
    edges: &[(&str, &str, &str)],
    patch: &[(&str, &str, &str)], // (qualified, column, value)
) -> AnalysisGraph {
    let (dir, _store) = fixture_db(nodes, edges);
    let conn = open_conn(dir.path());
    for (qn, col, val) in patch {
        conn.execute(
            &format!("UPDATE nodes SET {col} = ?1 WHERE qualified_name = ?2"),
            [val, qn],
        )
        .unwrap();
    }
    AnalysisGraph::load(&conn).unwrap()
}

const A: NodeRow = ("Function", "f.py::a", "a", "f.py", false);
const B: NodeRow = ("Function", "f.py::b", "b", "f.py", false);

#[test]
fn uncalled_function_is_entry() {
    let g = graph(&[A, B], &[("CALLS", "f.py::a", "f.py::b")], &[]);
    assert_eq!(qnames(&g, &detect_entry_points(&g, false)), ["f.py::a"]);
}

#[test]
fn called_function_with_route_decorator_is_entry() {
    let g = graph(
        &[A, B],
        &[("CALLS", "f.py::a", "f.py::b")],
        &[("f.py::b", "extra", r#"{"decorators":["app.get"]}"#)],
    );
    assert_eq!(
        qnames(&g, &detect_entry_points(&g, false)),
        ["f.py::a", "f.py::b"]
    );
}

#[test]
fn called_function_named_main_is_entry() {
    let g = graph(
        &[
            A,
            ("Function", "f.py::main", "main", "f.py", false),
            ("Function", "f.py::mainly", "mainly", "f.py", false),
        ],
        &[
            ("CALLS", "f.py::a", "f.py::main"),
            ("CALLS", "f.py::a", "f.py::mainly"),
        ],
        &[],
    );
    assert_eq!(
        qnames(&g, &detect_entry_points(&g, false)),
        ["f.py::a", "f.py::main"]
    );
}

#[test]
fn tests_skipped_unless_included() {
    let t = ("Test", "t.py::check", "check", "t.py", true);
    let g = graph(&[A, t], &[("CALLS", "f.py::a", "t.py::nothing")], &[]);
    assert_eq!(qnames(&g, &detect_entry_points(&g, false)), ["f.py::a"]);
    assert_eq!(
        qnames(&g, &detect_entry_points(&g, true)),
        ["f.py::a", "t.py::check"]
    );
}

#[test]
fn call_from_file_node_does_not_count_as_called() {
    let file = ("File", "f.py", "f.py", "f.py", false);
    let g = graph(&[file, B], &[("CALLS", "f.py", "f.py::b")], &[]);
    assert_eq!(qnames(&g, &detect_entry_points(&g, false)), ["f.py::b"]);
}

#[test]
fn java_called_only_from_tests_is_entry() {
    let t = ("Test", "T.java::t", "t", "T.java", true);
    let b = ("Function", "B.java::b", "b", "B.java", false);
    let g = graph(
        &[t, b],
        &[("CALLS", "T.java::t", "B.java::b")],
        &[("B.java::b", "language", "java")],
    );
    assert_eq!(qnames(&g, &detect_entry_points(&g, false)), ["B.java::b"]);
}

#[test]
fn php_boot_only_for_php() {
    let boot = ("Function", "f::boot", "boot", "f", false);
    let edges = [("CALLS", "f.py::a", "f::boot")];
    let py = graph(&[A, boot], &edges, &[("f::boot", "language", "python")]);
    assert_eq!(qnames(&py, &detect_entry_points(&py, false)), ["f.py::a"]);
    let php = graph(&[A, boot], &edges, &[("f::boot", "language", "php")]);
    assert_eq!(
        qnames(&php, &detect_entry_points(&php, false)),
        ["f.py::a", "f::boot"]
    );
}
