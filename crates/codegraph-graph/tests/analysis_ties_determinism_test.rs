// ABOUTME: Flows, hubs, bridges, knowledge gaps and surprising connections must break ties
// ABOUTME: by repo-relative keys, not by row id: same output under any root and row order.

use codegraph_graph::analysis::flows::{FLOW_MAX_DEPTH, trace_flows};
use codegraph_graph::analysis::insights::{
    bridge_nodes, hub_nodes, knowledge_gaps, surprising_connections,
};
use codegraph_graph::analysis::{AnalysisEdge, AnalysisGraph, AnalysisNode};

const FILES: usize = 6;
const FUNCTIONS: usize = 15;

/// (kind, repo-relative qualified name, file, line_start, line_end, community).
fn fixture_nodes() -> Vec<(&'static str, String, String, i64, i64, i64)> {
    let mut nodes = Vec::new();
    for k in 0..FILES {
        let file = format!("pkg/m{k}.py");
        nodes.push(("File", file.clone(), file.clone(), 1, 200, k as i64));
        for j in 0..FUNCTIONS {
            let start = 10 * j as i64 + 1;
            nodes.push((
                "Function",
                format!("{file}::fn{j}"),
                file.clone(),
                start,
                start + 4,
                k as i64,
            ));
        }
    }
    nodes
}

/// (kind, source, target, file, line), repo-relative. Every file has the same shape, so hubs,
/// bridges, flows and surprising edges all tie across files; fn4.. are isolated (66 > 50).
fn fixture_edges() -> Vec<(&'static str, String, String, String, i64)> {
    let mut edges = Vec::new();
    for k in 0..FILES {
        let file = format!("pkg/m{k}.py");
        let next = format!("pkg/m{}.py", (k + 1) % FILES);
        let q = |f: &str, j: usize| format!("{f}::fn{j}");
        for j in 0..FUNCTIONS {
            edges.push((
                "CONTAINS",
                file.clone(),
                q(&file, j),
                file.clone(),
                10 * j as i64 + 1,
            ));
        }
        edges.push(("CALLS", q(&file, 0), q(&file, 1), file.clone(), 2));
        edges.push(("CALLS", q(&file, 0), q(&file, 2), file.clone(), 3));
        edges.push(("CALLS", q(&file, 1), q(&next, 3), file.clone(), 12));
    }
    edges
}

/// Deterministic shuffle (LCG + Fisher-Yates) of `0..n`; seed 0 keeps the identity.
fn permutation(n: usize, seed: u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..n).collect();
    if seed == 0 {
        return order;
    }
    let mut state = seed;
    for i in (1..n).rev() {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        order.swap(i, (state >> 33) as usize % (i + 1));
    }
    order
}

/// The fixture rooted at `root`, with node and edge rows (ids) in permuted order.
fn graph(root: &str, seed: u64) -> AnalysisGraph {
    let fixture = fixture_nodes();
    let nodes = permutation(fixture.len(), seed)
        .into_iter()
        .enumerate()
        .map(|(row, i)| {
            let (kind, qn, file, start, end, community) = fixture[i].clone();
            AnalysisNode {
                id: row as i64 + 1,
                qualified: format!("{root}{qn}"),
                name: if kind == "File" {
                    format!("{root}{qn}")
                } else {
                    qn.rsplit("::").next().unwrap().to_string()
                },
                kind: kind.to_string(),
                file_path: format!("{root}{file}"),
                language: Some("python".to_string()),
                line_start: Some(start),
                line_end: Some(end),
                is_test: false,
                decorators: Vec::new(),
                community_id: Some(community),
                is_virtual: false,
                parent_name: None,
            }
        })
        .collect();
    let fixture = fixture_edges();
    let edges = permutation(fixture.len(), seed.wrapping_mul(31))
        .into_iter()
        .map(|i| {
            let (kind, source, target, file, line) = fixture[i].clone();
            AnalysisEdge {
                kind: kind.to_string(),
                source: format!("{root}{source}"),
                target: format!("{root}{target}"),
                file_path: format!("{root}{file}"),
                line,
                import_context: None,
                unresolved: false,
                evidence: None,
            }
        })
        .collect();
    AnalysisGraph::from_parts(nodes, edges)
}

/// Every analysis's output, made repo-relative, as one comparable list of lines.
fn outputs(root: &str, seed: u64) -> Vec<String> {
    let g = graph(root, seed);
    let rel = |s: &str| s.strip_prefix(root).unwrap_or(s).to_string();
    let mut out = Vec::new();
    for f in trace_flows(&g, FLOW_MAX_DEPTH) {
        let path: Vec<String> = f
            .path
            .iter()
            .map(|id| rel(&g.nodes[g.by_id(*id).unwrap()].qualified))
            .collect();
        out.push(format!(
            "flow {} {} {:?}",
            rel(&f.entry_point),
            f.criticality,
            path
        ));
    }
    for h in hub_nodes(&g) {
        out.push(format!("hub {} {}", rel(&h.qualified_name), h.total_degree));
    }
    for b in bridge_nodes(&g) {
        out.push(format!(
            "bridge {} {}",
            rel(&b.qualified_name),
            b.betweenness
        ));
    }
    let gaps = knowledge_gaps(&g, &[], None);
    for n in gaps.isolated_nodes {
        out.push(format!("isolated {} {}", rel(&n.qualified_name), n.degree));
    }
    for n in gaps.untested_hotspots {
        out.push(format!("hotspot {} {}", rel(&n.qualified_name), n.degree));
    }
    for s in surprising_connections(&g) {
        out.push(format!(
            "surprising {} {} {}",
            rel(&s.source_qualified),
            rel(&s.target_qualified),
            s.surprise_score
        ));
    }
    out
}

#[test]
fn analysis_ties_do_not_depend_on_root_or_row_order() {
    let reference = outputs("/w/a/", 0);
    for prefix in ["flow ", "hub ", "bridge ", "isolated ", "surprising "] {
        assert!(
            reference.iter().any(|l| l.starts_with(prefix)),
            "fixture produced no {prefix}rows"
        );
    }
    assert_eq!(
        reference
            .iter()
            .filter(|l| l.starts_with("isolated "))
            .count(),
        50
    );
    for (root, seed) in [
        ("/w/a/", 1),
        ("/w/a2/", 0),
        ("/tmp/a/much/longer/work/dir/", 7),
        ("/w/b/", 42),
        ("/w/c/", 1234),
        ("/w/d/", 99991),
    ] {
        assert_eq!(
            outputs(root, seed),
            reference,
            "root {root} seed {seed} changed the analysis output"
        );
    }
}

/// On a fresh build, the order is the CRG's insertion order: files by path, nodes in source order.
#[test]
fn analysis_ties_follow_file_and_source_order() {
    let out = outputs("/w/a/", 12345);
    let isolated: Vec<&String> = out.iter().filter(|l| l.starts_with("isolated ")).collect();
    assert_eq!(isolated[0], "isolated pkg/m0.py::fn4 1");
    assert_eq!(isolated[10], "isolated pkg/m0.py::fn14 1");
    assert_eq!(isolated[11], "isolated pkg/m1.py::fn4 1");
    let first_flow = out.iter().find(|l| l.starts_with("flow ")).unwrap();
    assert!(
        first_flow.starts_with("flow pkg/m0.py::fn0 "),
        "{first_flow}"
    );
    let first_hub = out.iter().find(|l| l.starts_with("hub ")).unwrap();
    assert!(first_hub.starts_with("hub pkg/m0.py::fn"), "{first_hub}");
}
