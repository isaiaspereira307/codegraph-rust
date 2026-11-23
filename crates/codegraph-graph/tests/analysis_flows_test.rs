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

// ---- trace, criticality and storage ----

use codegraph_graph::analysis::flows::{FLOW_MAX_DEPTH, store_flows, trace_flows};

fn chain(n: usize) -> (Vec<String>, Vec<(String, String)>) {
    let names: Vec<String> = (0..n)
        .map(|i| {
            if i == 0 {
                "main".into()
            } else {
                format!("f{i}")
            }
        })
        .collect();
    let edges = names
        .windows(2)
        .map(|w| (w[0].clone(), w[1].clone()))
        .collect();
    (names, edges)
}

fn flow_graph(
    names: &[&str],
    calls: &[(&str, &str)],
    extra: &[(&str, &str, &str)],
) -> AnalysisGraph {
    let qn = |n: &str| format!("f.py::{n}");
    let nodes: Vec<(String, String)> = names.iter().map(|n| (qn(n), n.to_string())).collect();
    let node_rows: Vec<NodeRow> = nodes
        .iter()
        .map(|(q, n)| ("Function", q.as_str(), n.as_str(), "f.py", false))
        .collect();
    let edges: Vec<(String, String, String)> = calls
        .iter()
        .map(|(a, b)| {
            (
                "CALLS".to_string(),
                qn(a),
                if b.contains("::") {
                    b.to_string()
                } else {
                    qn(b)
                },
            )
        })
        .chain(extra.iter().map(|(k, a, b)| (k.to_string(), qn(a), qn(b))))
        .collect();
    let edge_rows: Vec<(&str, &str, &str)> = edges
        .iter()
        .map(|(k, a, b)| (k.as_str(), a.as_str(), b.as_str()))
        .collect();
    let (dir, _store) = fixture_db(&node_rows, &edge_rows);
    AnalysisGraph::load(&open_conn(dir.path())).unwrap()
}

#[test]
fn linear_chain_traced_bfs() {
    let g = flow_graph(
        &["main", "a", "b", "c"],
        &[("main", "a"), ("a", "b"), ("a", "c")],
        &[],
    );
    let flows = trace_flows(&g, FLOW_MAX_DEPTH);
    assert_eq!(flows.len(), 1);
    let f = &flows[0];
    assert_eq!(f.name, "main");
    assert_eq!(f.path, g.nodes.iter().map(|n| n.id).collect::<Vec<_>>());
    assert_eq!((f.depth, f.node_count), (2, 4));
}

#[test]
fn recursion_terminates() {
    let g = flow_graph(
        &["main", "a"],
        &[("main", "a"), ("a", "a"), ("a", "main")],
        &[],
    );
    let flows = trace_flows(&g, FLOW_MAX_DEPTH);
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].path, vec![g.nodes[0].id, g.nodes[1].id]);
}

#[test]
fn depth_capped() {
    let (names, edges) = chain(20);
    let n: Vec<&str> = names.iter().map(String::as_str).collect();
    let e: Vec<(&str, &str)> = edges
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let g = flow_graph(&n, &e, &[]);
    let flows = trace_flows(&g, FLOW_MAX_DEPTH);
    assert_eq!((flows[0].node_count, flows[0].depth), (16, 15));
}

#[test]
fn single_node_entry_has_no_flow() {
    let g = flow_graph(&["main"], &[], &[]);
    assert!(trace_flows(&g, FLOW_MAX_DEPTH).is_empty());
}

#[test]
fn no_calls_no_flows() {
    let g = flow_graph(&["a", "b"], &[], &[]);
    assert!(trace_flows(&g, FLOW_MAX_DEPTH).is_empty());
}

#[test]
fn criticality_formula() {
    // main -> login_user -> c, c in another file, c has TESTED_BY, two unresolved call targets.
    let (dir, _store) = fixture_db(
        &[
            ("Function", "f.py::main", "main", "f.py", false),
            ("Function", "f.py::login_user", "login_user", "f.py", false),
            ("Function", "g.py::c", "c", "g.py", false),
            ("Test", "t.py::test_c", "test_c", "t.py", true),
        ],
        &[
            ("CALLS", "f.py::main", "f.py::login_user"),
            ("CALLS", "f.py::login_user", "g.py::c"),
            ("CALLS", "g.py::c", "ext1"),
            ("CALLS", "g.py::c", "ext2"),
            ("TESTED_BY", "g.py::c", "t.py::test_c"),
        ],
    );
    let g = AnalysisGraph::load(&open_conn(dir.path())).unwrap();
    let flows = trace_flows(&g, FLOW_MAX_DEPTH);
    assert_eq!(flows.len(), 1);
    // 0.30*0.25 + 0.20*0.4 + 0.25*(1/3) + 0.15*(2/3) + 0.10*0.2
    assert_eq!(flows[0].criticality, 0.3583);
}

#[test]
fn store_flows_writes_python_json_path() {
    let g = flow_graph(&["main", "a"], &[("main", "a")], &[]);
    let flows = trace_flows(&g, FLOW_MAX_DEPTH);
    let (dir, _s) = fixture_db(&[], &[]);
    let mut conn = open_conn(dir.path());
    let tx = conn.transaction().unwrap();
    assert_eq!(store_flows(&tx, &flows).unwrap(), 1);
    tx.commit().unwrap();
    let ids = flows[0].path.clone();
    let path: String = conn
        .query_row("SELECT path_json FROM flows", [], |r| r.get(0))
        .unwrap();
    assert_eq!(path, format!("[{}, {}]", ids[0], ids[1]));
    let positions: Vec<(i64, i64)> = conn
        .prepare("SELECT node_id, position FROM flow_memberships ORDER BY position")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(positions, vec![(ids[0], 0), (ids[1], 1)]);
}
