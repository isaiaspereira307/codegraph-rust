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
                import_context: None,
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

// ---- knowledge gaps and surprising connections ----

use codegraph_graph::analysis::insights::{knowledge_gaps, surprising_connections};
use codegraph_graph::graph_query::CommunityRow;

/// (id, kind, qualified, file, is_test, community)
type N<'a> = (i64, &'a str, &'a str, &'a str, bool, Option<i64>);

fn node(n: N) -> AnalysisNode {
    let (id, kind, qn, file, is_test, community_id) = n;
    AnalysisNode {
        id,
        qualified: qn.into(),
        name: qn.rsplit("::").next().unwrap().into(),
        kind: kind.into(),
        file_path: file.into(),
        language: None,
        line_start: None,
        line_end: None,
        is_test,
        decorators: vec![],
        community_id,
    }
}

fn edge(kind: &str, s: &str, t: &str) -> AnalysisEdge {
    AnalysisEdge {
        kind: kind.into(),
        source: s.into(),
        target: t.into(),
        file_path: String::new(),
        line: 0,
        import_context: None,
    }
}

fn import(s: &str, t: &str, line: i64, context: Option<&str>) -> AnalysisEdge {
    AnalysisEdge {
        line,
        import_context: context.map(str::to_string),
        ..edge("IMPORTS_FROM", s, t)
    }
}

fn mem_graph(nodes: &[N], edges: Vec<AnalysisEdge>) -> AnalysisGraph {
    AnalysisGraph::from_parts(nodes.iter().copied().map(node).collect(), edges)
}

fn community(id: i64, name: &str) -> CommunityRow {
    CommunityRow {
        id,
        name: name.into(),
        level: 0,
        cohesion: 0.0,
        size: 0,
        dominant_language: String::new(),
        description: String::new(),
        members: vec![],
    }
}

#[test]
fn gaps_on_empty_graph_are_empty() {
    let gaps = knowledge_gaps(&AnalysisGraph::default(), &[], None);
    assert!(gaps.isolated_nodes.is_empty());
    assert!(gaps.thin_communities.is_empty());
    assert!(gaps.untested_hotspots.is_empty());
    assert!(gaps.single_file_communities.is_empty());
    assert!(gaps.import_cycles.is_empty());
    assert!(surprising_connections(&AnalysisGraph::default()).is_empty());
}

#[test]
fn isolated_nodes_have_degree_at_most_one() {
    let g = mem_graph(
        &[
            (1, "Function", "f.py::lonely", "f.py", false, None),
            (2, "File", "f.py", "f.py", false, None),
            (3, "Function", "f.py::leaf", "f.py", false, None),
            (4, "Function", "f.py::mid", "f.py", false, None),
            (5, "Function", "f.py::other", "f.py", false, None),
        ],
        vec![
            edge("CALLS", "f.py::mid", "f.py::leaf"),
            edge("CALLS", "f.py::mid", "f.py::other"),
            edge("CALLS", "f.py::other", "ext"),
        ],
    );
    let gaps = knowledge_gaps(&g, &[], None);
    let got: Vec<(&str, usize)> = gaps
        .isolated_nodes
        .iter()
        .map(|n| (n.qualified_name.as_str(), n.degree))
        .collect();
    // The File node and nodes of degree 2 are not isolated; id order is kept.
    assert_eq!(got, [("f.py::lonely", 0), ("f.py::leaf", 1)]);
    assert_eq!(gaps.isolated_nodes[0].name, "lonely");
    assert_eq!(gaps.isolated_nodes[0].kind, "Function");
    assert_eq!(gaps.isolated_nodes[0].file, "f.py");
}

#[test]
fn thin_communities_count_non_file_members() {
    let g = mem_graph(
        &[
            (1, "File", "a.py", "a.py", false, Some(1)),
            (2, "Function", "a.py::x", "a.py", false, Some(1)),
            (3, "Function", "a.py::y", "a.py", false, Some(1)),
            (4, "Function", "b.py::p", "b.py", false, Some(2)),
            (5, "Function", "b.py::q", "b.py", false, Some(2)),
            (6, "Function", "c.py::r", "c.py", false, Some(2)),
        ],
        vec![],
    );
    // Communities are reported in id order whatever order the caller passes.
    let comms = [
        community(3, "empty"),
        community(2, "big"),
        community(1, "small"),
    ];
    let gaps = knowledge_gaps(&g, &comms, None);
    let got: Vec<(i64, &str, usize)> = gaps
        .thin_communities
        .iter()
        .map(|c| (c.community_id, c.name.as_str(), c.size))
        .collect();
    assert_eq!(got, [(1, "small", 2), (3, "empty", 0)]);
}

#[test]
fn untested_hotspots_skip_tested_and_test_nodes() {
    let mut nodes = vec![
        (1, "Function", "h.py::hot", "h.py", false, None),
        (2, "Function", "h.py::covered", "h.py", false, None),
        (3, "Function", "t.py::test_it", "t.py", true, None),
        (4, "Function", "h.py::hotter", "h.py", false, None),
    ];
    nodes.push((5, "Function", "h.py::sink", "h.py", false, None));
    let mut edges = Vec::new();
    for _ in 0..5 {
        edges.push(edge("CALLS", "h.py::hot", "h.py::sink"));
        edges.push(edge("CALLS", "t.py::test_it", "h.py::sink"));
        edges.push(edge("CALLS", "h.py::covered", "ext"));
    }
    for _ in 0..6 {
        edges.push(edge("CALLS", "h.py::hotter", "ext"));
    }
    edges.push(edge("TESTED_BY", "h.py::covered", "t.py::test_it"));
    let g = mem_graph(&nodes, edges);
    let gaps = knowledge_gaps(&g, &[], None);
    let got: Vec<(&str, usize)> = gaps
        .untested_hotspots
        .iter()
        .map(|n| (n.qualified_name.as_str(), n.degree))
        .collect();
    // sink: degree 10, untested; hotter 6; hot 5. covered is tested, test_it is a test.
    assert_eq!(
        got,
        [("h.py::sink", 10), ("h.py::hotter", 6), ("h.py::hot", 5)]
    );
}

#[test]
fn single_file_communities_need_three_members() {
    let g = mem_graph(
        &[
            (1, "Function", "a.py::a1", "a.py", false, Some(1)),
            (2, "Function", "a.py::a2", "a.py", false, Some(1)),
            (3, "Function", "a.py::a3", "a.py", false, Some(1)),
            (4, "Function", "b.py::b1", "b.py", false, Some(2)),
            (5, "Function", "c.py::c1", "c.py", false, Some(2)),
            (6, "Function", "c.py::c2", "c.py", false, Some(2)),
            (7, "Function", "d.py::d1", "d.py", false, Some(3)),
            (8, "Function", "d.py::d2", "d.py", false, Some(3)),
        ],
        vec![],
    );
    let comms = [
        community(1, "one"),
        community(2, "two"),
        community(3, "three"),
    ];
    let gaps = knowledge_gaps(&g, &comms, None);
    let got: Vec<(i64, &str, usize, &str)> = gaps
        .single_file_communities
        .iter()
        .map(|c| (c.community_id, c.name.as_str(), c.size, c.file.as_str()))
        .collect();
    assert_eq!(got, [(1, "one", 3, "a.py")]);
}

#[test]
fn import_cycle_shortest_loop() {
    let files = [
        (1, "File", "/r/a.py", "/r/a.py", false, None),
        (2, "File", "/r/b.py", "/r/b.py", false, None),
        (3, "File", "/r/c.py", "/r/c.py", false, None),
        (4, "File", "/r/x.py", "/r/x.py", false, None),
        (5, "File", "/r/y.py", "/r/y.py", false, None),
        (6, "File", "/r/t.py", "/r/t.py", false, None),
    ];
    let edges = vec![
        import("/r/a.py", "/r/b.py", 3, None),
        import("/r/b.py", "/r/c.py", 5, None),
        import("/r/c.py", "/r/a.py", 7, None),
        import("/r/a.py", "/r/c.py", 9, None),
        import("/r/a.py", "/r/c.py", 4, None),
        import("/r/a.py", "/r/a.py", 1, None),
        // Only deferred imports: a cycle that cannot fail at load time.
        import("/r/x.py", "/r/y.py", 2, Some("deferred")),
        import("/r/y.py", "/r/x.py", 2, None),
        // Type-only and parent-package imports never count.
        import("/r/t.py", "/r/a.py", 1, Some("type_only")),
        import("/r/a.py", "/r/t.py", 1, Some("parent_package")),
        // A target that is not a file is not followed.
        import("/r/c.py", "pkg.mod", 1, None),
    ];
    let g = mem_graph(&files, edges);
    let gaps = knowledge_gaps(&g, &[], Some("/r"));
    let cycles = &gaps.import_cycles;
    assert_eq!(cycles.len(), 2);
    let first = &cycles[0];
    // BFS from the alphabetically smallest file, neighbours sorted: a -> c -> a beats a -> b -> c.
    assert_eq!(first.name, "a.py -> c.py -> a.py");
    assert_eq!(first.size, 3);
    assert!(first.load_time);
    assert_eq!(first.files, ["a.py", "b.py", "c.py"]);
    assert_eq!(first.loop_length, 2);
    let hops: Vec<(&str, i64, &str)> = first
        .cycle
        .iter()
        .map(|h| (h.file.as_str(), h.line, h.imports.as_str()))
        .collect();
    assert_eq!(hops, [("a.py", 4, "c.py"), ("c.py", 7, "a.py")]);
    let second = &cycles[1];
    assert_eq!(second.name, "x.py -> y.py -> x.py");
    assert!(!second.load_time);
    assert_eq!(second.size, 2);
    assert_eq!(second.cycle[0].line, 2);
}

#[test]
fn surprise_reasons_and_score() {
    let mut nodes = vec![
        (1, "Function", "a.py::caller", "a.py", false, Some(1)),
        (2, "Function", "b.rs::callee", "b.rs", false, Some(2)),
        (3, "Type", "a.py::T", "a.py", false, Some(2)),
        (4, "Function", "t.py::test_x", "t.py", true, Some(1)),
        (5, "Function", "a.py::hub", "a.py", false, Some(1)),
        (6, "File", "a.py", "a.py", false, Some(1)),
    ];
    let leaves: Vec<String> = (0..10).map(|i| format!("a.py::l{i}")).collect();
    for (i, qn) in leaves.iter().enumerate() {
        nodes.push((
            10 + i as i64,
            "Function",
            qn.as_str(),
            "a.py",
            false,
            Some(1),
        ));
    }
    let mut edges = vec![
        edge("CALLS", "a.py::caller", "b.rs::callee"),
        edge("CALLS", "a.py::T", "a.py::caller"),
        edge("CALLS", "t.py::test_x", "a.py::caller"),
        edge("CALLS", "a.py::caller", "a.py::hub"),
        edge("CALLS", "a.py", "b.rs::callee"),
        edge("CALLS", "b.rs::callee", "ext"),
    ];
    for qn in &leaves {
        edges.push(edge("REFERENCES", qn, "a.py::hub"));
    }
    let g = mem_graph(&nodes, edges);
    let rows = surprising_connections(&g);
    let got: Vec<(String, &str, f64, Vec<&str>)> = rows
        .iter()
        .map(|r| {
            (
                r.source.clone(),
                r.target.as_str(),
                r.surprise_score,
                r.reasons.clone(),
            )
        })
        .collect();
    let mut expected = vec![
        (
            "caller".to_string(),
            "callee",
            0.5,
            vec!["cross-community", "cross-language"],
        ),
        (
            "T".to_string(),
            "caller",
            0.45,
            vec!["cross-community", "unusual-edge-kind"],
        ),
    ];
    for i in 0..10 {
        expected.push((format!("l{i}"), "hub", 0.2, vec!["peripheral-to-hub"]));
    }
    expected.push((
        "test_x".to_string(),
        "caller",
        0.15,
        vec!["cross-test-boundary"],
    ));
    assert_eq!(got, expected);
    assert_eq!(rows[0].source_qualified, "a.py::caller");
    assert_eq!(rows[0].target_qualified, "b.rs::callee");
    assert_eq!(rows[0].edge_kind, "CALLS");
    assert_eq!(rows[0].source_community, Some(1));
    assert_eq!(rows[0].target_community, Some(2));
}

#[tokio::test]
async fn gaps_and_surprises_on_crg_written_db() {
    let (dir, store) = fixture_db(
        &[f("f.py::a"), f("f.py::b")],
        &[("CALLS", "f.py::a", "f.py::b")],
    );
    let _ = dir;
    let q = GraphQuery::new(store);
    let gaps = q.knowledge_gaps().await.unwrap();
    assert_eq!(gaps.isolated_nodes.len(), 2);
    assert!(q.surprising_connections().await.unwrap().is_empty());
}

#[test]
fn import_context_is_read_from_edge_extra() {
    let (dir, _store) = fixture_db(
        &[],
        &[
            ("IMPORTS_FROM", "a.ts", "b.ts"),
            ("IMPORTS_FROM", "b.ts", "a.ts"),
        ],
    );
    let conn = open_conn(dir.path());
    conn.execute(
        "UPDATE edges SET extra = '{\"import_context\": \"type_only\"}' WHERE source_qualified = 'a.ts'",
        [],
    )
    .unwrap();
    let g = AnalysisGraph::load(&conn).unwrap();
    assert_eq!(g.edges[0].import_context.as_deref(), Some("type_only"));
    assert_eq!(g.edges[1].import_context, None);
}

// ---- suggested questions ----

use codegraph_graph::analysis::insights::suggested_questions;

#[test]
fn questions_in_crg_order_with_literal_text() {
    let mut nodes: Vec<N> = vec![
        (1, "Function", "a.py::c", "a.py", false, Some(1)),
        (2, "Function", "a.py::h", "a.py", false, Some(1)),
        (3, "Function", "a.py::t", "a.py", true, Some(1)),
    ];
    let leaves = ["l1", "l2", "l3", "l4", "l5"];
    let qns: Vec<String> = leaves.iter().map(|l| format!("b.py::{l}")).collect();
    for (i, qn) in qns.iter().enumerate() {
        nodes.push((
            4 + i as i64,
            "Function",
            qn.as_str(),
            "b.py",
            false,
            Some(2),
        ));
    }
    let mut edges = vec![
        edge("CALLS", "a.py::c", "a.py::h"),
        edge("TESTED_BY", "a.py::c", "a.py::t"),
    ];
    edges.extend(qns.iter().map(|q| edge("CALLS", "a.py::h", q)));
    let g = mem_graph(&nodes, edges);
    let communities = [
        community(1, "core"),
        community(2, "leaves"),
        community(3, "tiny"),
    ];
    let got: Vec<(String, String, String, String)> = suggested_questions(&g, &communities)
        .into_iter()
        .map(|q| (q.category.into(), q.question, q.target, q.priority.into()))
        .collect();
    let row = |c: &str, q: &str, t: &str, p: &str| {
        (c.to_string(), q.to_string(), t.to_string(), p.to_string())
    };
    let want = vec![
        row(
            "bridge_node",
            "'h' is a critical connector between multiple code regions. Is it adequately tested and documented?",
            "a.py::h",
            "high",
        ),
        row(
            "hub_risk",
            "Hub node 'h' has 6 connections but no direct test coverage. Should it be tested?",
            "a.py::h",
            "high",
        ),
        row(
            "hub_risk",
            "Hub node 't' has 1 connections but no direct test coverage. Should it be tested?",
            "a.py::t",
            "high",
        ),
        row(
            "surprising_connection",
            "'h' (community 1) calls 'l1' (community 2). Is this coupling intentional?",
            "a.py::h",
            "medium",
        ),
        row(
            "surprising_connection",
            "'h' (community 1) calls 'l2' (community 2). Is this coupling intentional?",
            "a.py::h",
            "medium",
        ),
        row(
            "surprising_connection",
            "'h' (community 1) calls 'l3' (community 2). Is this coupling intentional?",
            "a.py::h",
            "medium",
        ),
        row(
            "thin_community",
            "Community 'tiny' has only 0 member(s). Should it be merged with a neighbor?",
            "community:3",
            "low",
        ),
        row(
            "untested_hotspot",
            "'h' has 6 connections but no test coverage. Is this a risk?",
            "a.py::h",
            "medium",
        ),
    ];
    assert_eq!(got, want);
}
