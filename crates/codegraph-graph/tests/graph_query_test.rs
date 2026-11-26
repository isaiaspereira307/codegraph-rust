// ABOUTME: Verifies the petgraph query layer against a small graph with known answers.
// ABOUTME: One test per graph tool plus snapshot reload and legacy id forms.

use codegraph_core::{CodeNode, EdgeType, Language, Location, NodeType};
use codegraph_graph::CodeEdge;
use codegraph_graph::graph_query::GraphQuery;
use codegraph_graph::sqlite_store::SqliteGraphStore;
use rusqlite::Connection;
use std::path::Path;

fn id(n: u128) -> String {
    uuid::Uuid::from_u128(n).to_string()
}

fn node(n: u128, name: &str, file: &str, complexity: f32) -> CodeNode {
    let mut node = CodeNode::new(
        name,
        Some(NodeType::Function),
        Some(Language::Python),
        Location {
            file_path: file.to_string(),
            line: n as u32,
            column: 0,
            end_line: Some(n as u32 + 2),
            end_column: None,
        },
    )
    .with_complexity(complexity);
    node.id = uuid::Uuid::from_u128(n);
    node
}

fn calls(e: u128, from: u128, to: u128) -> CodeEdge {
    let mut edge = CodeEdge::new(
        uuid::Uuid::from_u128(from),
        uuid::Uuid::from_u128(to),
        EdgeType::Calls,
    );
    edge.id = uuid::Uuid::from_u128(e);
    edge
}

/// a↔b cycle; chain c1→c2→c3→c4; hub h called by d1..d6; big (complexity 12) called by d1.
async fn fixture(root: &Path) -> (SqliteGraphStore, GraphQuery) {
    let store = SqliteGraphStore::open(root).unwrap();
    let mut nodes = vec![
        node(1, "a", "cycle.py", 1.0),
        node(2, "b", "cycle.py", 1.0),
        node(11, "c1", "chain.py", 1.0),
        node(12, "c2", "chain.py", 1.0),
        node(13, "c3", "chain.py", 1.0),
        node(14, "c4", "chain.py", 1.0),
        node(20, "h", "hub.py", 1.0),
        node(30, "big", "big.py", 12.0),
    ];
    for d in 21..=26 {
        nodes.push(node(d, &format!("d{}", d - 20), "deps.py", 1.0));
    }
    store.upsert_nodes_batch(&nodes).await.unwrap();
    let mut edges = vec![
        calls(100, 1, 2),
        calls(101, 2, 1),
        calls(102, 11, 12),
        calls(103, 12, 13),
        calls(104, 13, 14),
        calls(105, 21, 30),
    ];
    for d in 21..=26u128 {
        edges.push(calls(200 + d, d, 20));
    }
    store.upsert_edges_batch(&edges).await.unwrap();
    store.finalize("full").await.unwrap();
    let query = GraphQuery::new(store.clone());
    (store, query)
}

#[tokio::test]
async fn find_nodes_by_name_returns_uuid_and_location() {
    let dir = tempfile::tempdir().unwrap();
    let (_, q) = fixture(dir.path()).await;
    let found = q.find_nodes_by_name("BIG", 10).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, id(30));
    assert_eq!(found[0].kind.as_deref(), Some("Function"));
    let loc = found[0].location.as_ref().unwrap();
    assert_eq!(loc.file_path, dir.path().join("big.py").to_string_lossy());
    assert_eq!((loc.start_line, loc.end_line), (Some(30), Some(32)));
    // Matches on file path too; sorted by name, limited.
    let chain = q.find_nodes_by_name("chain.py", 2).await.unwrap();
    let names: Vec<_> = chain.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, vec!["c1", "c2"]);
}

#[tokio::test]
async fn transitive_dependencies_follow_out_edges_by_depth() {
    let dir = tempfile::tempdir().unwrap();
    let (_, q) = fixture(dir.path()).await;
    let deps = q
        .get_transitive_dependencies(&id(11), "Calls", 2)
        .await
        .unwrap();
    let got: Vec<_> = deps
        .iter()
        .map(|d| (d.name.as_str(), d.dependency_depth))
        .collect();
    assert_eq!(got, vec![("c2", Some(1)), ("c3", Some(2))]);
    assert_eq!(deps[0].kind.as_deref(), Some("Function"));
}

#[tokio::test]
async fn reverse_dependencies_follow_in_edges_and_clamp_depth() {
    let dir = tempfile::tempdir().unwrap();
    let (_, q) = fixture(dir.path()).await;
    let deps = q
        .get_reverse_dependencies(&id(14), "CALLS", 99)
        .await
        .unwrap();
    let got: Vec<_> = deps
        .iter()
        .map(|d| (d.name.as_str(), d.dependent_depth))
        .collect();
    // Depth 99 is out of range and falls back to 3.
    assert_eq!(got, vec![("c3", Some(1)), ("c2", Some(2)), ("c1", Some(3))]);
}

#[tokio::test]
async fn circular_dependencies_report_each_pair_once() {
    let dir = tempfile::tempdir().unwrap();
    let (_, q) = fixture(dir.path()).await;
    let cycles = q.detect_circular_dependencies("calls").await.unwrap();
    assert_eq!(cycles.len(), 1);
    assert_eq!(
        (cycles[0].node1_id.clone(), cycles[0].node2_id.clone()),
        (id(1), id(2))
    );
    assert_eq!(cycles[0].node1.name, "a");
    assert_eq!(cycles[0].dependency_type, "CALLS");
    assert!(
        q.detect_circular_dependencies("imports")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn call_chain_has_depth_path_and_callers() {
    let dir = tempfile::tempdir().unwrap();
    let (_, q) = fixture(dir.path()).await;
    let chain = q.trace_call_chain(&id(11), 5).await.unwrap();
    let got: Vec<_> = chain
        .iter()
        .map(|c| (c.name.as_str(), c.call_depth, c.call_path.clone().unwrap()))
        .collect();
    assert_eq!(
        got,
        vec![
            ("c2", Some(1), vec!["c1".to_string(), "c2".into()]),
            ("c3", Some(2), vec!["c1".into(), "c2".into(), "c3".into()]),
            (
                "c4",
                Some(3),
                vec!["c1".into(), "c2".into(), "c3".into(), "c4".into()]
            ),
        ]
    );
    let callers = chain[1].called_by.as_ref().unwrap();
    assert_eq!(callers.len(), 1);
    assert_eq!(
        (callers[0].id.as_str(), callers[0].name.as_str()),
        (id(12).as_str(), "c2")
    );
}

#[tokio::test]
async fn coupling_metrics_classify_stability() {
    let dir = tempfile::tempdir().unwrap();
    let (_, q) = fixture(dir.path()).await;
    let hub = q.calculate_coupling_metrics(&id(20)).await.unwrap();
    assert_eq!(hub.metrics.afferent_coupling, 6);
    assert_eq!(hub.metrics.efferent_coupling, 0);
    assert_eq!(hub.metrics.instability, 0.0);
    assert_eq!(hub.metrics.coupling_category, "stable");
    assert!(hub.metrics.is_stable);
    assert_eq!(hub.dependents.len(), 6);

    let d1 = q.calculate_coupling_metrics(&id(21)).await.unwrap();
    assert_eq!(
        (d1.metrics.efferent_coupling, d1.metrics.total_coupling),
        (2, 2)
    );
    assert_eq!(d1.metrics.instability, 1.0);
    assert_eq!(d1.metrics.coupling_category, "unstable");

    let c2 = q.calculate_coupling_metrics(&id(12)).await.unwrap();
    assert_eq!(c2.metrics.instability, 0.5);
    assert_eq!(c2.metrics.coupling_category, "balanced");
    assert_eq!(c2.node.name, "c2");

    assert!(q.calculate_coupling_metrics("missing").await.is_err());
}

#[tokio::test]
async fn hub_nodes_rank_by_total_degree() {
    let dir = tempfile::tempdir().unwrap();
    let (_, q) = fixture(dir.path()).await;
    let hubs = q.get_hub_nodes(5).await.unwrap();
    assert_eq!(hubs.len(), 1);
    assert_eq!(hubs[0].node_id, id(20));
    assert_eq!(
        (
            hubs[0].afferent_degree,
            hubs[0].efferent_degree,
            hubs[0].total_degree
        ),
        (6, 0, 6)
    );
    assert_eq!(hubs[0].incoming_by_type.len(), 1);
    assert_eq!(hubs[0].incoming_by_type[0].edge_type, "CALLS");
    assert_eq!(hubs[0].incoming_by_type[0].count, 6);
    assert!(hubs[0].node.content.is_none());
    // min_degree <= 0 falls back to 5.
    assert_eq!(q.get_hub_nodes(0).await.unwrap().len(), 1);
    assert_eq!(q.get_hub_nodes(2).await.unwrap()[0].node_id, id(20));
}

#[tokio::test]
async fn complexity_hotspots_rank_by_risk() {
    let dir = tempfile::tempdir().unwrap();
    let (_, q) = fixture(dir.path()).await;
    let spots = q.get_complexity_hotspots(5.0, 20).await.unwrap();
    assert_eq!(spots.len(), 1);
    let big = &spots[0];
    assert_eq!(
        (big.id.as_str(), big.name.as_str()),
        (id(30).as_str(), "big")
    );
    assert_eq!(big.complexity, 12.0);
    assert_eq!((big.afferent_coupling, big.efferent_coupling), (1, 0));
    assert_eq!(big.instability, 0.0);
    assert_eq!(big.risk_score, 24.0);
    assert_eq!(
        q.get_complexity_hotspots(0.0, 3).await.unwrap().len(),
        1,
        "min <= 0 -> 5"
    );
    assert_eq!(q.count_nodes_for_project().await.unwrap(), 14);
}

#[tokio::test]
async fn accepts_legacy_record_id_forms() {
    let dir = tempfile::tempdir().unwrap();
    let (_, q) = fixture(dir.path()).await;
    let qn = format!("{}::c1", dir.path().join("chain.py").to_string_lossy());
    for form in [
        id(11),
        format!("nodes:`{}`", id(11)),
        format!("nodes:⟨{}⟩", id(11)),
        format!("nodes:{}", id(11)),
        qn,
    ] {
        let deps = q
            .get_transitive_dependencies(&form, "calls", 1)
            .await
            .unwrap();
        assert_eq!(deps.len(), 1, "form {form}");
        assert_eq!(deps[0].name, "c2");
    }
    assert!(
        q.get_transitive_dependencies("nodes:`nope`", "calls", 1)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn snapshot_reloads_after_write() {
    let dir = tempfile::tempdir().unwrap();
    let (store, q) = fixture(dir.path()).await;
    assert_eq!(q.get_hub_nodes(5).await.unwrap()[0].afferent_degree, 6);

    // A write through the same store handle.
    store
        .upsert_nodes_batch(&[node(27, "d7", "deps.py", 1.0)])
        .await
        .unwrap();
    store
        .upsert_edges_batch(&[calls(227, 27, 20)])
        .await
        .unwrap();
    assert_eq!(q.get_hub_nodes(5).await.unwrap()[0].afferent_degree, 7);

    // A write from another connection (another process in production).
    let other = Connection::open(dir.path().join(".code-review-graph/graph.db")).unwrap();
    other
        .execute(
            "DELETE FROM edges WHERE source_qualified LIKE '%::d7' OR source_qualified LIKE '%::d6'",
            [],
        )
        .unwrap();
    assert_eq!(q.get_hub_nodes(5).await.unwrap()[0].afferent_degree, 5);
}

#[tokio::test]
async fn graph_tools_skip_dangling_targets() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = fixture(dir.path()).await;
    // c4 calls an unresolved `print` many times; text targets are not graph nodes.
    let mut edges = Vec::new();
    for line in 0..6 {
        let mut e = CodeEdge::unresolved(uuid::Uuid::from_u128(14), "print", EdgeType::Calls)
            .with_metadata("line".into(), line.to_string());
        e.set_deterministic_id("p");
        edges.push(e);
    }
    store.upsert_edges_batch(&edges).await.unwrap();
    store.finalize("full").await.unwrap();
    let q = GraphQuery::new(store.clone());

    let deps = q
        .get_reverse_dependencies(&id(14), "CALLS", 3)
        .await
        .unwrap();
    assert_eq!(deps.len(), 3);
    let out = q
        .get_transitive_dependencies(&id(14), "CALLS", 3)
        .await
        .unwrap();
    assert!(out.is_empty());
    assert!(q.find_nodes_by_name("print", 10).await.unwrap().is_empty());
    let hubs = q.get_hub_nodes(5).await.unwrap();
    assert_eq!(hubs.len(), 1);
    assert_eq!(hubs[0].node_id, id(20));
    assert_eq!(
        q.detect_circular_dependencies("CALLS").await.unwrap().len(),
        1
    );
}

/// A CRG-style File node (named by its absolute path) for `file` under `root`.
fn file_node(n: u128, root: &Path, file: &str) -> CodeNode {
    let path = root.join(file).to_string_lossy().to_string();
    let mut node = CodeNode::new(
        path.as_str(),
        Some(NodeType::Other("File".into())),
        Some(Language::Python),
        Location {
            file_path: file.to_string(),
            line: 1,
            column: 0,
            end_line: Some(3),
            end_column: None,
        },
    );
    node.id = uuid::Uuid::from_u128(n);
    node
}

#[tokio::test]
async fn hub_nodes_exclude_file_nodes() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = fixture(dir.path()).await;
    // lib.py is imported by d1..d6: a File hub, which code-review-graph never reports.
    store
        .upsert_nodes_batch(&[file_node(40, dir.path(), "lib.py")])
        .await
        .unwrap();
    let edges: Vec<_> = (21..=26u128)
        .map(|d| {
            let mut e = CodeEdge::new(
                uuid::Uuid::from_u128(d),
                uuid::Uuid::from_u128(40),
                EdgeType::Imports,
            );
            e.id = uuid::Uuid::from_u128(300 + d);
            e
        })
        .collect();
    store.upsert_edges_batch(&edges).await.unwrap();
    store.finalize("full").await.unwrap();
    let q = GraphQuery::new(store.clone());
    let hubs: Vec<_> = q
        .get_hub_nodes(5)
        .await
        .unwrap()
        .into_iter()
        .map(|h| h.node_id)
        .collect();
    assert_eq!(hubs, vec![id(20)]);
}

#[tokio::test]
async fn find_nodes_by_name_ranks_name_matches_and_symbols_first() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = fixture(dir.path()).await;
    // big.py's File node is named by its absolute path, which sorts before any symbol name.
    let mut extra = vec![file_node(41, dir.path(), "big.py")];
    extra.push(node(50, "zchain", "other.py", 1.0));
    store.upsert_nodes_batch(&extra).await.unwrap();
    store.finalize("full").await.unwrap();
    let q = GraphQuery::new(store.clone());
    let big: Vec<_> = q
        .find_nodes_by_name("big", 10)
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.id)
        .collect();
    assert_eq!(big, vec![id(30), id(41)]);
    // A name match outranks the c1..c4 whose path alone contains "chain".
    let chain: Vec<_> = q
        .find_nodes_by_name("chain", 2)
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert_eq!(chain, vec!["zchain", "c1"]);
}

#[tokio::test]
async fn search_nodes_matches_stems_and_camel_tokens_name_first() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteGraphStore::open(dir.path()).unwrap();
    let parse = node(60, "parse_file", "reader.py", 1.0);
    let hashed = node(61, "hashPassword", "auth.py", 1.0);
    // Name hit on "reader".
    let by_name = node(62, "reader", "other.py", 1.0);
    // Path-only hit: "reader" appears only in this node's file path.
    let by_path = node(63, "helper", "reader_utils.py", 1.0);
    store
        .upsert_nodes_batch(&[parse, hashed, by_name, by_path])
        .await
        .unwrap();
    store.finalize("full").await.unwrap();
    let q = GraphQuery::new(store.clone());

    // Porter stemming: "parsing" reaches "parse_file".
    let found = q.search_nodes("parsing", 10).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, id(60));

    // camelCase tokens: "password" reaches "hashPassword".
    let found = q.search_nodes("password", 10).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, id(61));

    // Name hits rank above path-only hits.
    let found = q.search_nodes("reader", 10).await.unwrap();
    assert_eq!(found.len(), 3);
    assert_eq!(found[0].id, id(62));

    // Operators and quotes are plain text; blank input finds nothing; limit 0 uses the default.
    assert!(q.search_nodes("\"\" OR NEAR(", 10).await.is_ok());
    assert!(q.search_nodes("   ", 10).await.unwrap().is_empty());
    assert_eq!(q.search_nodes("password", 0).await.unwrap().len(), 1);
}

#[tokio::test]
async fn graph_stats_count_kinds_files_languages_and_last_build() {
    let dir = tempfile::tempdir().unwrap();
    let (_store, q) = fixture(dir.path()).await;

    let stats = q.graph_stats().await.unwrap();
    assert_eq!(stats.total_nodes, 14);
    assert_eq!(stats.nodes_by_kind.get("Function"), Some(&14));
    assert_eq!(stats.total_edges, 12);
    assert_eq!(stats.edges_by_kind.get("CALLS"), Some(&12));
    // cycle.py, chain.py, hub.py, big.py, deps.py
    assert_eq!(stats.files, 5);
    assert_eq!(stats.languages, vec!["python".to_string()]);
    assert!(stats.last_updated.is_some(), "finalize writes last_updated");

    // An empty database reports zeros and no build time.
    let empty_dir = tempfile::tempdir().unwrap();
    let empty = SqliteGraphStore::open(empty_dir.path()).unwrap();
    let stats = GraphQuery::new(empty).graph_stats().await.unwrap();
    assert_eq!(stats.total_nodes, 0);
    assert_eq!(stats.total_edges, 0);
    assert_eq!(stats.files, 0);
    assert!(stats.languages.is_empty());
    assert_eq!(stats.last_updated, None);
}

fn spanned(n: u128, name: &str, file: &str, start: u32, end: u32) -> CodeNode {
    let mut node = CodeNode::new(
        name,
        Some(NodeType::Function),
        Some(Language::Python),
        Location {
            file_path: file.to_string(),
            line: start,
            column: 0,
            end_line: Some(end),
            end_column: None,
        },
    );
    node.id = uuid::Uuid::from_u128(n);
    node
}

#[tokio::test]
async fn find_large_functions_applies_line_threshold_longest_first() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteGraphStore::open(dir.path()).unwrap();
    store
        .upsert_nodes_batch(&[
            spanned(70, "exact", "a.py", 1, 50), // 50 lines: at the threshold
            spanned(71, "short", "a.py", 10, 40), // 31 lines: below
            spanned(72, "huge", "b.py", 1, 200), // 200 lines
            spanned(73, "medium", "b.py", 5, 104), // 100 lines
        ])
        .await
        .unwrap();
    store.finalize("full").await.unwrap();
    let q = GraphQuery::new(store.clone());

    // Default threshold is 50 lines; longest first.
    let found = q.find_large_functions(0, 0).await.unwrap();
    let names: Vec<_> = found.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["huge", "medium", "exact"]);
    assert_eq!(found[0].line_count, 200);
    assert_eq!(found[2].line_count, 50);
    assert_eq!(found[0].id, id(72));

    // Explicit threshold of 100 keeps lines >= 100.
    let found = q.find_large_functions(100, 10).await.unwrap();
    let names: Vec<_> = found.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, vec!["huge", "medium"]);

    // Limit applies after ordering.
    let found = q.find_large_functions(1, 1).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "huge");
}

#[tokio::test]
async fn query_graph_follows_call_edges_and_counts_unresolved_targets() {
    use codegraph_graph::graph_query::QueryPattern;

    let dir = tempfile::tempdir().unwrap();
    let (store, _) = fixture(dir.path()).await;
    // c1 calls an unresolved `print`: text target, not a node.
    let mut print = CodeEdge::unresolved(uuid::Uuid::from_u128(11), "print", EdgeType::Calls);
    print.set_deterministic_id("print");
    store.upsert_edges_batch(&[print]).await.unwrap();
    store.finalize("full").await.unwrap();
    let q = GraphQuery::new(store.clone());

    // h has six callers d1..d6, all resolved.
    let callers = q
        .query_graph(QueryPattern::CallersOf, &id(20), 0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(callers.pattern, "callers_of");
    assert_eq!(callers.target.name, "h");
    assert_eq!(callers.total, 6);
    assert_eq!(callers.results.len(), 6);
    assert_eq!(callers.unresolved, 0);

    // The limit truncates results but not the total.
    let limited = q
        .query_graph(QueryPattern::CallersOf, &id(20), 2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((limited.results.len(), limited.total), (2, 6));

    // d1 calls big and h, sorted by name.
    let callees = q
        .query_graph(QueryPattern::CalleesOf, &id(21), 0)
        .await
        .unwrap()
        .unwrap();
    let names: Vec<_> = callees.results.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, vec!["big", "h"]);

    // c1's text target is ignored and counted.
    let c1 = q
        .query_graph(QueryPattern::CalleesOf, &id(11), 0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(c1.total, 1);
    assert_eq!(c1.results[0].name, "c2");
    assert_eq!(c1.unresolved, 1);

    // No import or test edges in this fixture: empty, not an error.
    let tests = q
        .query_graph(QueryPattern::TestsFor, &id(20), 0)
        .await
        .unwrap()
        .unwrap();
    assert!(tests.results.is_empty());

    // Unknown node and unknown pattern name.
    assert!(
        q.query_graph(QueryPattern::CallersOf, &id(999), 0)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        QueryPattern::parse("callers_of"),
        Some(QueryPattern::CallersOf)
    );
    assert_eq!(QueryPattern::parse("callers"), None);
}

#[tokio::test]
async fn traverse_graph_walks_by_direction_depth_and_budget() {
    use codegraph_graph::graph_query::TraverseDirection;

    let dir = tempfile::tempdir().unwrap();
    let (_store, q) = fixture(dir.path()).await;

    // c1 → c2 → c3 → c4 along CALLS.
    let out = q
        .traverse_graph(&id(11), TraverseDirection::Out, Some("Calls"), 2, 0)
        .await
        .unwrap()
        .unwrap();
    let got: Vec<_> = out
        .nodes
        .iter()
        .map(|n| (n.node.name.as_str(), n.depth))
        .collect();
    assert_eq!(got, vec![("c2", 1), ("c3", 2)]);
    assert!(!out.truncated);

    // Walking in from c4 reaches c3, c2, c1 (depth 3), nearest first.
    let inward = q
        .traverse_graph(&id(14), TraverseDirection::In, Some("CALLS"), 10, 0)
        .await
        .unwrap()
        .unwrap();
    let names: Vec<_> = inward.nodes.iter().map(|n| n.node.name.as_str()).collect();
    assert_eq!(names, vec!["c3", "c2", "c1"]);

    // Both directions from c2: c1 (in) and c3 (out) at depth 1, then c4 at depth 2.
    let both = q
        .traverse_graph(&id(12), TraverseDirection::Both, Some("CALLS"), 2, 0)
        .await
        .unwrap()
        .unwrap();
    let got: Vec<_> = both
        .nodes
        .iter()
        .map(|n| (n.node.name.as_str(), n.depth))
        .collect();
    assert_eq!(got, vec![("c1", 1), ("c3", 1), ("c4", 2)]);

    // The node budget stops the walk and says so.
    let capped = q
        .traverse_graph(&id(14), TraverseDirection::In, Some("CALLS"), 10, 2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(capped.nodes.len(), 2);
    assert!(capped.truncated);

    // Depth outside 1..=10 falls back to the default of 3; unknown nodes are None.
    let deflt = q
        .traverse_graph(&id(11), TraverseDirection::Out, Some("CALLS"), 99, 0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(deflt.nodes.len(), 3);
    assert!(
        q.traverse_graph(&id(999), TraverseDirection::Out, None, 1, 0)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn impact_radius_walks_dependents_files_and_tests() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = fixture(dir.path()).await;
    // A test node (kind Test) calls c2, so finalize derives TESTED_BY c2 -> test.
    let mut test = CodeNode::new(
        "test_c2",
        Some(NodeType::Other("Test".to_string())),
        Some(Language::Python),
        Location {
            file_path: "test_chain.py".to_string(),
            line: 1,
            column: 0,
            end_line: Some(3),
            end_column: None,
        },
    );
    test.id = uuid::Uuid::from_u128(80);
    store.upsert_nodes_batch(&[test]).await.unwrap();
    store
        .upsert_edges_batch(&[calls(400, 80, 12)])
        .await
        .unwrap();
    store.finalize("full").await.unwrap();
    let q = GraphQuery::new(store.clone());

    // c4's dependents: c3 at depth 1, c2 at depth 2 (depth 2 stops there).
    let radius = q.get_impact_radius(&[id(14)], &[], 2, 0).await.unwrap();
    let got: Vec<_> = radius
        .impacted
        .iter()
        .map(|n| (n.node.name.as_str(), n.depth))
        .collect();
    assert_eq!(got, vec![("c3", 1), ("c2", 2)]);
    assert_eq!(radius.changed.len(), 1);
    assert!(!radius.truncated);
    // c2 sits at the last level, so the test covering it is a third hop: not reported (the
    // TESTED_BY hop counts against the depth, as in code-review-graph).
    assert!(radius.tests.is_empty());
    // Only chain.py is touched, and it holds the changed node: no file outside the change.
    assert!(radius.affected_files.is_empty());

    // One more level and the test of c2 is in reach.
    let deeper = q.get_impact_radius(&[id(14)], &[], 3, 0).await.unwrap();
    assert_eq!(deeper.tests.len(), 1);
    assert_eq!(deeper.tests[0].name, "test_c2");

    // Full depth reaches c1 too, and the test that calls c2 is a dependent as well.
    let full = q.get_impact_radius(&[id(14)], &[], 10, 0).await.unwrap();
    assert_eq!(full.impacted.len(), 4);
    assert!(full.impacted.iter().any(|n| n.node.name == "test_c2"));
    // Only the test's own file: chain.py holds the changed node.
    assert_eq!(full.affected_files.len(), 1);
    assert!(full.affected_files[0].ends_with("test_chain.py"));

    // Changing big.py: its file node set is seeded, and d1 depends on big.
    let by_file = q
        .get_impact_radius(&[], &["big.py".to_string()], 3, 0)
        .await
        .unwrap();
    assert_eq!(by_file.changed.len(), 1);
    assert_eq!(by_file.changed[0].name, "big");
    let names: Vec<_> = by_file
        .impacted
        .iter()
        .map(|n| n.node.name.as_str())
        .collect();
    assert_eq!(names, vec!["d1"]);

    // The node budget stops the walk and says so; unknown ids are ignored.
    let capped = q
        .get_impact_radius(&[id(14), id(999)], &[], 10, 1)
        .await
        .unwrap();
    assert_eq!(capped.changed.len(), 1);
    assert_eq!(capped.impacted.len(), 1);
    assert!(capped.truncated);
}

#[tokio::test]
async fn nodes_at_lines_matches_overlapping_spans_by_file_suffix() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteGraphStore::open(dir.path()).unwrap();
    store
        .upsert_nodes_batch(&[
            spanned(90, "top", "pkg/mod.py", 1, 10),
            spanned(91, "later", "pkg/mod.py", 12, 20),
            spanned(92, "other", "pkg/other.py", 1, 10),
        ])
        .await
        .unwrap();
    store.finalize("full").await.unwrap();
    let q = GraphQuery::new(store.clone());

    // Line 10 touches `top` only; a repo-relative path matches the stored absolute path.
    let hits = q
        .nodes_at_lines(&[("pkg/mod.py".to_string(), vec![(10, 10)])])
        .await
        .unwrap();
    let names: Vec<_> = hits.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, vec!["top"]);

    // A range spanning both functions in mod.py, and nothing from other.py.
    let hits = q
        .nodes_at_lines(&[("pkg/mod.py".to_string(), vec![(9, 13)])])
        .await
        .unwrap();
    let names: Vec<_> = hits.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, vec!["later", "top"]);

    // Lines outside every span, or other files, match nothing.
    assert!(
        q.nodes_at_lines(&[("pkg/mod.py".to_string(), vec![(30, 40)])])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        q.nodes_at_lines(&[("missing.py".to_string(), vec![(1, 5)])])
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn node_coverage_counts_dependents_and_tests() {
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = fixture(dir.path()).await;
    let mut test = CodeNode::new(
        "test_h",
        Some(NodeType::Other("Test".to_string())),
        Some(Language::Python),
        Location {
            file_path: "test_hub.py".to_string(),
            line: 1,
            column: 0,
            end_line: Some(2),
            end_column: None,
        },
    );
    test.id = uuid::Uuid::from_u128(81);
    store.upsert_nodes_batch(&[test]).await.unwrap();
    store
        .upsert_edges_batch(&[calls(401, 81, 20)])
        .await
        .unwrap();
    store.finalize("full").await.unwrap();
    let q = GraphQuery::new(store.clone());

    let cov = q.node_coverage(&[id(20), id(30), id(999)]).await.unwrap();
    // h: six callers (d1..d6) plus the test, which is also a dependent; big: d1, no test.
    assert_eq!(cov.len(), 2);
    assert_eq!((cov[0].dependents, cov[0].tests), (7, 1));
    assert_eq!((cov[1].dependents, cov[1].tests), (1, 0));
}

async fn run_all_analysis_tools(q: &GraphQuery) -> String {
    let hubs = q.hub_nodes().await.unwrap();
    q.bridge_nodes().await.unwrap();
    q.knowledge_gaps().await.unwrap();
    q.suggested_questions().await.unwrap();
    q.surprising_connections().await.unwrap();
    format!("{hubs:?}")
}

#[tokio::test]
async fn analysis_graph_is_loaded_once_across_tools() {
    let dir = tempfile::tempdir().unwrap();
    let (_store, q) = fixture(dir.path()).await;
    run_all_analysis_tools(&q).await;
    run_all_analysis_tools(&q).await;
    assert_eq!(q.analysis_graph_loads(), 1);
}

#[tokio::test]
async fn analysis_graph_reloads_after_a_write() {
    let dir = tempfile::tempdir().unwrap();
    let (store, q) = fixture(dir.path()).await;
    let before = run_all_analysis_tools(&q).await;
    assert_eq!(q.analysis_graph_loads(), 1);

    store
        .upsert_nodes_batch(&[node(27, "d7", "deps.py", 1.0)])
        .await
        .unwrap();
    store
        .upsert_edges_batch(&[calls(227, 27, 20)])
        .await
        .unwrap();
    let after = run_all_analysis_tools(&q).await;
    assert_eq!(q.analysis_graph_loads(), 2);
    assert_ne!(before, after);
}

#[tokio::test]
async fn analysis_graph_reloads_after_another_connection_writes() {
    let dir = tempfile::tempdir().unwrap();
    let (_store, q) = fixture(dir.path()).await;
    let before = run_all_analysis_tools(&q).await;

    let other = Connection::open(dir.path().join(".code-review-graph/graph.db")).unwrap();
    other
        .execute("DELETE FROM edges WHERE source_qualified LIKE '%::d6'", [])
        .unwrap();
    let after = run_all_analysis_tools(&q).await;
    assert_eq!(q.analysis_graph_loads(), 2);
    assert_ne!(before, after);
}
