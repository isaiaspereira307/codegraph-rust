// ABOUTME: Tests hub and bridge insights: degree counting, Brandes betweenness and sampling.
// ABOUTME: Fixtures are hand-built SQLite graphs without cg_* rows, like a CRG-written database.

#[path = "common/analysis_fixture.rs"]
mod analysis_fixture;

use analysis_fixture::{NodeRow, fixture_db, open_conn};
use codegraph_graph::analysis::insights::{betweenness, bridge_nodes, hub_nodes};
use codegraph_graph::analysis::{AnalysisEdge, AnalysisGraph, AnalysisNode};
use codegraph_graph::graph_query::GraphQuery;

fn graph(nodes: &[NodeRow], edges: &[(&str, &str, &str)]) -> AnalysisGraph {
    let (dir, _store) = fixture_db(nodes, edges);
    AnalysisGraph::load(&open_conn(dir.path())).unwrap()
}

fn f(qn: &'static str) -> NodeRow<'static> {
    (
        "Function",
        qn,
        qn.rsplit("::").next().unwrap(),
        "f.py",
        false,
    )
}

#[test]
fn hub_degrees_count_parallel_edges() {
    let g = graph(
        &[
            f("f.py::a"),
            f("f.py::b"),
            f("f.py::c"),
            f("f.py::idle"),
            ("File", "f.py", "f.py", "f.py", false),
        ],
        &[
            ("CALLS", "f.py::a", "f.py::b"),
            ("CALLS", "f.py::a", "f.py::b"),
            ("IMPORTS_FROM", "f.py::b", "f.py::c"),
            ("CALLS", "f.py", "f.py::a"),
            ("CALLS", "f.py::c", "text_target"),
        ],
    );
    let hubs = hub_nodes(&g);
    let names: Vec<(&str, usize, usize, usize)> = hubs
        .iter()
        .map(|h| (h.name.as_str(), h.in_degree, h.out_degree, h.total_degree))
        .collect();
    // b: in 2, out 1; a: in 1 (from the File), out 2; c: in 1, out 1. File and idle are absent.
    assert_eq!(names, [("a", 1, 2, 3), ("b", 2, 1, 3), ("c", 1, 1, 2)]);
}

#[test]
fn betweenness_path_graph() {
    let scores = betweenness(3, &[vec![1], vec![2], vec![]], None);
    assert_eq!(scores, vec![0.0, 0.5, 0.0]);
}

#[test]
fn betweenness_tiny_graphs_are_zero() {
    assert!(betweenness(0, &[], None).is_empty());
    assert_eq!(betweenness(2, &[vec![1], vec![]], None), vec![0.0, 0.0]);
}

#[test]
fn bridge_between_two_clusters_ranks_first() {
    let edges: Vec<(&str, &str, &str)> = vec![
        ("CALLS", "f.py::a0", "f.py::a1"),
        ("CALLS", "f.py::a1", "f.py::a0"),
        ("CALLS", "f.py::a1", "f.py::x"),
        ("CALLS", "f.py::x", "f.py::a1"),
        ("CALLS", "f.py::x", "f.py::b0"),
        ("CALLS", "f.py::b0", "f.py::x"),
        ("CALLS", "f.py::b0", "f.py::b1"),
        ("CALLS", "f.py::b1", "f.py::b0"),
    ];
    let g = graph(
        &[
            f("f.py::a0"),
            f("f.py::a1"),
            f("f.py::x"),
            f("f.py::b0"),
            f("f.py::b1"),
        ],
        &edges,
    );
    let bridges = bridge_nodes(&g);
    assert_eq!(bridges[0].name, "x");
    assert!(bridges[0].betweenness > bridges[1].betweenness);
    assert!(bridges.iter().all(|b| b.betweenness > 0.0));
}

#[test]
fn text_endpoints_are_vertices_but_never_reported() {
    let g = graph(
        &[f("f.py::a"), f("f.py::b")],
        &[("CALLS", "f.py::a", "ext"), ("CALLS", "ext", "f.py::b")],
    );
    let bridges = bridge_nodes(&g);
    assert!(bridges.iter().all(|b| b.qualified_name != "ext"));
}

#[tokio::test]
async fn bridges_on_crg_written_db() {
    let (dir, store) = fixture_db(
        &[f("f.py::a"), f("f.py::m"), f("f.py::b")],
        &[
            ("CALLS", "f.py::a", "f.py::m"),
            ("CALLS", "f.py::m", "f.py::b"),
        ],
    );
    let _ = dir;
    let q = GraphQuery::new(store);
    let bridges = q.bridge_nodes().await.unwrap();
    assert_eq!(bridges.len(), 1);
    assert_eq!(bridges[0].name, "m");
    assert_eq!(bridges[0].community_id, None);
    let hubs = q.hub_nodes().await.unwrap();
    assert_eq!(hubs[0].name, "m");
}

#[test]
fn sampled_betweenness_is_deterministic() {
    // 6000 nodes in a ring with chords: above the 5000-vertex sampling threshold.
    let n = 6000;
    let nodes: Vec<AnalysisNode> = (0..n)
        .map(|i| AnalysisNode {
            id: i as i64 + 1,
            qualified: format!("f.py::n{i}"),
            name: format!("n{i}"),
            kind: "Function".into(),
            file_path: "f.py".into(),
            language: None,
            line_start: None,
            line_end: None,
            is_test: false,
            decorators: vec![],
            community_id: None,
        })
        .collect();
    let mut edges = Vec::new();
    for i in 0..n {
        for step in [1usize, 37] {
            edges.push(AnalysisEdge {
                kind: "CALLS".into(),
                source: format!("f.py::n{i}"),
                target: format!("f.py::n{}", (i + step) % n),
                file_path: String::new(),
                line: 0,
            });
        }
    }
    let g = AnalysisGraph::from_parts(nodes, edges);
    let first = bridge_nodes(&g);
    let second = bridge_nodes(&g);
    assert!(!first.is_empty());
    let key = |v: &[codegraph_graph::analysis::insights::BridgeRow]| -> Vec<(String, u64)> {
        v.iter()
            .map(|b| (b.qualified_name.clone(), b.betweenness.to_bits()))
            .collect()
    };
    assert_eq!(key(&first), key(&second));
}
