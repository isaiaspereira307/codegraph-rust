// ABOUTME: Tests community detection on hand-built graphs: weights, test reassignment, fallback, split.
// ABOUTME: Graphs are SQLite fixtures without cg_* rows, like a database written by the CRG.

#[path = "common/analysis_fixture.rs"]
mod analysis_fixture;

use analysis_fixture::{NodeRow, fixture_db, open_conn};
use codegraph_graph::analysis::AnalysisGraph;
use codegraph_graph::analysis::communities::{
    Community, community_graph, compute_cohesion, detect_communities, split_oversized,
};

fn graph(nodes: &[NodeRow], edges: &[(&str, &str, &str)]) -> AnalysisGraph {
    let (dir, _store) = fixture_db(nodes, edges);
    AnalysisGraph::load(&open_conn(dir.path())).unwrap()
}

/// Function nodes `{prefix}0..{prefix}{n}` in `{file}`.
fn funcs(prefix: &str, n: usize, file: &'static str) -> Vec<(String, String, &'static str)> {
    (0..n)
        .map(|i| (format!("{file}::{prefix}{i}"), format!("{prefix}{i}"), file))
        .collect()
}

fn rows<'a>(spec: &'a [(String, String, &'static str)]) -> Vec<NodeRow<'a>> {
    spec.iter()
        .map(|(qn, name, file)| ("Function", qn.as_str(), name.as_str(), *file, false))
        .collect()
}

fn dense(prefix: &str, n: usize, file: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            out.push((
                format!("{file}::{prefix}{i}"),
                format!("{file}::{prefix}{j}"),
            ));
        }
    }
    out
}

fn calls<'a>(pairs: &'a [(String, String)]) -> Vec<(&'a str, &'a str, &'a str)> {
    pairs
        .iter()
        .map(|(a, b)| ("CALLS", a.as_str(), b.as_str()))
        .collect()
}

fn community_of<'a>(comms: &'a [Community], qn: &str) -> Option<&'a Community> {
    comms.iter().find(|c| c.members.iter().any(|m| m == qn))
}

#[test]
fn two_modules_two_communities() {
    let a = funcs("a", 4, "a.py");
    let b = funcs("b", 4, "b.py");
    let mut nodes = rows(&a);
    nodes.extend(rows(&b));
    let mut pairs = dense("a", 4, "a.py");
    pairs.extend(dense("b", 4, "b.py"));
    pairs.push(("a.py::a3".into(), "b.py::b0".into()));
    let g = graph(&nodes, &calls(&pairs));
    let comms = detect_communities(&g, 2);
    assert_eq!(comms.len(), 2);
    assert!(
        comms
            .iter()
            .all(|c| c.size == 4 && c.detector == "leiden" && c.level == 0)
    );
    assert_ne!(
        community_of(&comms, "a.py::a0").unwrap().members,
        community_of(&comms, "b.py::b0").unwrap().members
    );
    assert_eq!(comms[0].description, "Community of 4 nodes");
}

#[test]
fn file_nodes_excluded() {
    let a = funcs("a", 3, "a.py");
    let mut nodes = rows(&a);
    nodes.push(("File", "a.py", "a.py", "a.py", false));
    let mut pairs = dense("a", 3, "a.py");
    pairs.push(("a.py".into(), "a.py::a0".into()));
    let g = graph(&nodes, &calls(&pairs));
    let comms = detect_communities(&g, 2);
    assert!(comms.iter().all(|c| !c.members.iter().any(|m| m == "a.py")));
}

#[test]
fn first_edge_kind_sets_weight() {
    let a = funcs("a", 2, "a.py");
    let nodes = rows(&a);
    let g = graph(
        &nodes,
        &[
            ("IMPORTS_FROM", "a.py::a0", "a.py::a1"),
            ("CALLS", "a.py::a1", "a.py::a0"),
            ("INHERITS", "a.py::a0", "ext"),
        ],
    );
    let (vertices, wg) = community_graph(&g);
    assert_eq!(vertices, vec![0, 1]);
    assert_eq!(wg.weight(0, 1), Some(0.5));
}

#[test]
fn test_node_follows_its_subjects() {
    let a = funcs("a", 4, "a.py");
    let b = funcs("b", 4, "b.py");
    let mut nodes = rows(&a);
    nodes.extend(rows(&b));
    nodes.push(("Test", "t.py::test_b", "test_b", "t.py", true));
    let mut pairs = dense("a", 4, "a.py");
    pairs.extend(dense("b", 4, "b.py"));
    pairs.push(("a.py::a3".into(), "b.py::b0".into()));
    let mut edges = calls(&pairs);
    // Strong call links pull the test toward module A; its tested subjects live in B.
    edges.extend([
        ("CALLS", "t.py::test_b", "a.py::a0"),
        ("CALLS", "t.py::test_b", "a.py::a1"),
        ("CALLS", "t.py::test_b", "a.py::a2"),
        ("TESTED_BY", "b.py::b0", "t.py::test_b"),
        ("TESTED_BY", "b.py::b1", "t.py::test_b"),
    ]);
    let g = graph(&nodes, &edges);
    let comms = detect_communities(&g, 2);
    let test_comm = community_of(&comms, "t.py::test_b").unwrap();
    assert!(test_comm.members.iter().any(|m| m == "b.py::b0"));
    assert!(test_comm.members.iter().any(|m| m == "b.py::b1"));
}

#[test]
fn singletons_dropped() {
    let a = funcs("a", 3, "a.py");
    let mut nodes = rows(&a);
    nodes.push(("Function", "z.py::lonely", "lonely", "z.py", false));
    let pairs = dense("a", 3, "a.py");
    let g = graph(&nodes, &calls(&pairs));
    let comms = detect_communities(&g, 2);
    assert!(community_of(&comms, "z.py::lonely").is_none());
    assert_eq!(comms.len(), 1);
}

#[test]
fn edgeless_graph_uses_directory_fallback() {
    let mut spec = funcs("a", 3, "/p/d1/a.py");
    spec.extend(funcs("b", 3, "/p/d2/b.py"));
    let g = graph(&rows(&spec), &[]);
    let comms = detect_communities(&g, 2);
    assert_eq!(comms.len(), 2);
    assert!(comms.iter().all(|c| c.detector == "file" && c.size == 3));
    assert_eq!(comms[0].description, "Directory-based community: d1");
    assert_eq!(comms[1].description, "Directory-based community: d2");
}

#[test]
fn text_targets_ignored() {
    let a = funcs("a", 3, "a.py");
    let nodes = rows(&a);
    let mut pairs = dense("a", 3, "a.py");
    pairs.push(("a.py::a0".into(), "ext_function".into()));
    let g = graph(&nodes, &calls(&pairs));
    let comms = detect_communities(&g, 2);
    assert_eq!(comms.len(), 1);
    assert!(!comms[0].members.iter().any(|m| m == "ext_function"));
    assert_eq!(comms[0].size, 3);
}

#[test]
fn cohesion_counts_external_both_sides() {
    let edges = |pairs: &[(&str, &str)]| -> Vec<codegraph_graph::analysis::AnalysisEdge> {
        pairs
            .iter()
            .map(|(s, t)| codegraph_graph::analysis::AnalysisEdge {
                kind: "CALLS".into(),
                source: s.to_string(),
                target: t.to_string(),
                file_path: String::new(),
                line: 0,
                import_context: None,
                unresolved: false,
                evidence: None,
            })
            .collect()
    };
    let all = edges(&[
        ("a", "b"),
        ("a", "b"),
        ("a", "c"),
        ("c", "d"),
        ("a", "text"),
    ]);
    let members = vec![
        vec!["a".to_string(), "b".to_string()],
        vec!["c".to_string(), "d".to_string()],
    ];
    let cohesion = compute_cohesion(&members, &all);
    // A: internal 2 (a-b twice), external 2 (a-c, a-text); B: internal 1, external 1.
    assert_eq!(cohesion, vec![0.5, 0.5]);
    let lonely = compute_cohesion(&[vec!["z".to_string()]], &all);
    assert_eq!(lonely, vec![0.0]);
}

#[test]
fn oversized_community_is_split() {
    // Four dense groups of ten, weakly chained, all forced into one community.
    let mut spec = Vec::new();
    let mut pairs = Vec::new();
    for (i, prefix) in ["a", "b", "c", "d"].iter().enumerate() {
        let file: &'static str = ["a.py", "b.py", "c.py", "d.py"][i];
        spec.extend(funcs(prefix, 10, file));
        pairs.extend(dense(prefix, 10, file));
    }
    for (x, y) in [
        ("a.py::a9", "b.py::b0"),
        ("b.py::b9", "c.py::c0"),
        ("c.py::c9", "d.py::d0"),
    ] {
        pairs.push((x.into(), y.into()));
    }
    let g = graph(&rows(&spec), &calls(&pairs));
    let all: Vec<String> = g.nodes.iter().map(|n| n.qualified.clone()).collect();
    let big = Community {
        name: "big".into(),
        level: 0,
        cohesion: 1.0,
        size: all.len(),
        dominant_language: "python".into(),
        description: "Community of 40 nodes".into(),
        members: all,
        detector: "leiden",
    };
    let split = split_oversized(vec![big], &g);
    assert!(split.len() > 1, "expected a split, got {}", split.len());
    assert_eq!(split.iter().map(|c| c.size).sum::<usize>(), 40);
    assert!(
        split
            .iter()
            .all(|c| c.level == 1 && c.description == "Split from big")
    );
    assert!(split.iter().all(|c| c.dominant_language == "python"));
}

#[test]
fn small_community_is_not_split() {
    let spec = funcs("a", 6, "a.py");
    let pairs = dense("a", 6, "a.py");
    let g = graph(&rows(&spec), &calls(&pairs));
    let comms = detect_communities(&g, 2);
    assert_eq!(comms.len(), 1);
    assert_eq!(comms[0].level, 0);
}

#[test]
fn empty_graph_no_communities() {
    let g = graph(&[], &[]);
    assert!(detect_communities(&g, 2).is_empty());
}

// ---- storage and community reads ----

use codegraph_graph::analysis::{PostprocessLevel, run_postprocess};
use codegraph_graph::graph_query::GraphQuery;

#[test]
fn store_sets_community_id_and_detector() {
    let a = funcs("a", 4, "a.py");
    let b = funcs("b", 4, "b.py");
    let mut nodes = rows(&a);
    nodes.extend(rows(&b));
    let mut pairs = dense("a", 4, "a.py");
    pairs.extend(dense("b", 4, "b.py"));
    pairs.push(("a.py::a3".into(), "b.py::b0".into()));
    let (dir, _store) = fixture_db(&nodes, &calls(&pairs));
    let mut conn = open_conn(dir.path());
    let stats = run_postprocess(&mut conn, PostprocessLevel::Full).unwrap();
    assert_eq!(stats.communities, 2);
    let stored: i64 = conn
        .query_row("SELECT count(*) FROM communities", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stored, 2);
    let assigned: i64 = conn
        .query_row(
            "SELECT count(*) FROM nodes WHERE community_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(assigned, 8);
    let detector: String = conn
        .query_row(
            "SELECT value FROM metadata WHERE key = 'community_detector'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(detector, "leiden");
    // A second run replaces, never appends, and keeps node assignments.
    let before: Vec<(String, Option<i64>)> = conn
        .prepare("SELECT qualified_name, community_id FROM nodes ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    run_postprocess(&mut conn, PostprocessLevel::Full).unwrap();
    let after: Vec<(String, Option<i64>)> = conn
        .prepare("SELECT qualified_name, community_id FROM nodes ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let comm_count: i64 = conn
        .query_row("SELECT count(*) FROM communities", [], |r| r.get(0))
        .unwrap();
    assert_eq!(comm_count, 2);
    // An unchanged graph keeps the very same community id for every node.
    assert_eq!(before, after);
}

/// Three hand-written communities: billing (12), orders (12), test-billing (2).
fn community_db() -> (tempfile::TempDir, GraphQuery) {
    let mut spec = funcs("b", 12, "billing/b.py");
    spec.extend(funcs("o", 12, "orders/o.py"));
    spec.extend(funcs("t", 2, "tests/t.py"));
    let mut edges: Vec<(String, String)> = Vec::new();
    for i in 0..11 {
        edges.push((format!("billing/b.py::b{i}"), format!("orders/o.py::o{i}")));
    }
    for i in 0..12 {
        edges.push((
            format!("tests/t.py::t{}", i % 2),
            format!("billing/b.py::b{i}"),
        ));
    }
    let (dir, store) = fixture_db(&rows(&spec), &calls(&edges));
    let conn = open_conn(dir.path());
    for (id, name, size, cohesion, prefix) in [
        (1, "billing", 12, 0.9, "billing/"),
        (2, "orders", 12, 0.5, "orders/"),
        (3, "test-billing", 2, 0.1, "tests/"),
    ] {
        conn.execute(
            "INSERT INTO communities (id, name, level, cohesion, size, dominant_language, description) \
             VALUES (?1, ?2, 0, ?3, ?4, 'python', 'd')",
            rusqlite::params![id, name, cohesion, size],
        )
        .unwrap();
        conn.execute(
            "UPDATE nodes SET community_id = ?1 WHERE file_path LIKE ?2",
            rusqlite::params![id, format!("{prefix}%")],
        )
        .unwrap();
    }
    (dir, GraphQuery::new(store))
}

#[tokio::test]
async fn list_communities_sorting_and_filtering() {
    let (_d, q) = community_db();
    let by_size = q.list_communities("size", 0).await.unwrap();
    assert_eq!(
        by_size.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["billing", "orders", "test-billing"]
    );
    assert_eq!(by_size[0].members.len(), 12);
    assert_eq!(by_size[0].members[0], "billing/b.py::b0");
    let by_cohesion = q.list_communities("cohesion", 0).await.unwrap();
    assert_eq!(by_cohesion[1].name, "orders");
    let by_name = q.list_communities("name", 0).await.unwrap();
    assert_eq!(by_name[0].name, "billing");
    assert_eq!(by_name[2].name, "test-billing");
    let big = q.list_communities("bogus", 5).await.unwrap();
    assert_eq!(big.len(), 2);
}

#[tokio::test]
async fn get_community_by_name_substring() {
    let (_d, q) = community_db();
    let c = q.get_community(None, Some("BILL")).await.unwrap().unwrap();
    assert_eq!(c.name, "billing");
    let by_id = q
        .get_community(Some(2), Some("ignored"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(by_id.name, "orders");
    assert!(q.get_community(None, Some("zzz")).await.unwrap().is_none());
    let members = q.community_members(2).await.unwrap();
    assert_eq!(members.len(), 12);
    assert_eq!(members[0].name, "o0");
}

#[tokio::test]
async fn overview_warns_on_high_coupling_skips_test_communities() {
    let (_d, q) = community_db();
    let overview = q.architecture_overview().await.unwrap();
    assert_eq!(overview.communities.len(), 3);
    // 11 billing->orders edges and 12 tests->billing edges; tests are skipped in warnings.
    assert_eq!(overview.cross_community_edges.len(), 23);
    assert_eq!(
        overview.warnings,
        ["High coupling (11 edges) between 'billing' and 'orders'"]
    );
}

#[tokio::test]
async fn overview_ignores_tested_by_edges() {
    let (dir, q) = community_db();
    let conn = open_conn(dir.path());
    for i in 0..12 {
        conn.execute(
            "INSERT INTO edges (kind, source_qualified, target_qualified, file_path, line, updated_at) \
             VALUES ('TESTED_BY', ?1, ?2, '', 1, 0)",
            rusqlite::params![format!("billing/b.py::b{i}"), format!("orders/o.py::o{i}")],
        )
        .unwrap();
    }
    let overview = q.architecture_overview().await.unwrap();
    assert_eq!(overview.cross_community_edges.len(), 23);
}
