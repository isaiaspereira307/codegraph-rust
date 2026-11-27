// ABOUTME: Community detection must not depend on the work-dir path or on node/edge row order.
// ABOUTME: Uses the php-imports parity probe graph under two roots and several row permutations.

use codegraph_graph::analysis::communities::detect_communities;
use codegraph_graph::analysis::{AnalysisEdge, AnalysisGraph, AnalysisNode};

/// (kind, repo-relative qualified name, line_start, line_end, is_test), as the Rust indexer
/// stores the `scripts/parity/probes/php-imports` probe.
const NODES: &[(&str, &str, i64, i64, bool)] = &[
    ("File", "Demo/App.php", 1, 26, false),
    ("Class", "Demo/App.php::App", 7, 25, false),
    ("Function", "Demo/App.php::App.run", 9, 12, false),
    ("Function", "Demo/App.php::App.build", 14, 19, false),
    ("Function", "Demo/App.php::App.offset", 21, 24, false),
    ("File", "Demo/Helper.php", 1, 17, false),
    ("Class", "Demo/Helper.php::Helper", 5, 16, false),
    ("Function", "Demo/Helper.php::Helper.make", 7, 10, false),
    ("Function", "Demo/Helper.php::Helper.twice", 12, 15, false),
    ("File", "tests/AppTest.php", 1, 31, true),
    ("Class", "tests/AppTest.php::AppTest", 11, 30, true),
    ("Test", "tests/AppTest.php::AppTest.testRun", 13, 17, true),
    (
        "Test",
        "tests/AppTest.php::AppTest.buildsFresh",
        20,
        24,
        true,
    ),
    (
        "Function",
        "tests/AppTest.php::AppTest.helperValue",
        26,
        29,
        true,
    ),
];

/// (kind, source, target, file, line); `@` marks a repo-relative path (prefixed with the root).
const EDGES: &[(&str, &str, &str, &str, i64)] = &[
    (
        "IMPORTS_FROM",
        "@Demo/App.php",
        "@Demo/Helper.php",
        "Demo/App.php",
        5,
    ),
    (
        "CONTAINS",
        "@Demo/App.php",
        "@Demo/App.php::App",
        "Demo/App.php",
        7,
    ),
    (
        "CONTAINS",
        "@Demo/App.php::App",
        "@Demo/App.php::App.run",
        "Demo/App.php",
        9,
    ),
    (
        "CALLS",
        "@Demo/App.php::App.run",
        "@Demo/Helper.php::Helper.twice",
        "Demo/App.php",
        11,
    ),
    (
        "CONTAINS",
        "@Demo/App.php::App",
        "@Demo/App.php::App.build",
        "Demo/App.php",
        14,
    ),
    (
        "CALLS",
        "@Demo/App.php::App.build",
        "@Demo/Helper.php::Helper.make",
        "Demo/App.php",
        16,
    ),
    (
        "CALLS",
        "@Demo/App.php::App.build",
        "@Demo/Helper.php::Helper",
        "Demo/App.php",
        17,
    ),
    (
        "CALLS",
        "@Demo/App.php::App.build",
        "@Demo/App.php::App.offset",
        "Demo/App.php",
        18,
    ),
    (
        "CALLS",
        "@Demo/App.php::App.build",
        "@Demo/Helper.php::Helper.twice",
        "Demo/App.php",
        18,
    ),
    (
        "CONTAINS",
        "@Demo/App.php::App",
        "@Demo/App.php::App.offset",
        "Demo/App.php",
        21,
    ),
    (
        "CONTAINS",
        "@Demo/Helper.php",
        "@Demo/Helper.php::Helper",
        "Demo/Helper.php",
        5,
    ),
    (
        "CONTAINS",
        "@Demo/Helper.php::Helper",
        "@Demo/Helper.php::Helper.make",
        "Demo/Helper.php",
        7,
    ),
    (
        "CALLS",
        "@Demo/Helper.php::Helper.make",
        "self",
        "Demo/Helper.php",
        9,
    ),
    (
        "CONTAINS",
        "@Demo/Helper.php::Helper",
        "@Demo/Helper.php::Helper.twice",
        "Demo/Helper.php",
        12,
    ),
    (
        "IMPORTS_FROM",
        "@tests/AppTest.php",
        "@Demo/Helper.php",
        "tests/AppTest.php",
        5,
    ),
    (
        "IMPORTS_FROM",
        "@tests/AppTest.php",
        "@Demo/App.php",
        "tests/AppTest.php",
        6,
    ),
    (
        "IMPORTS_FROM",
        "@tests/AppTest.php",
        "Demo\\Missing",
        "tests/AppTest.php",
        6,
    ),
    (
        "CONTAINS",
        "@tests/AppTest.php",
        "@tests/AppTest.php::AppTest",
        "tests/AppTest.php",
        11,
    ),
    (
        "CONTAINS",
        "@tests/AppTest.php::AppTest",
        "@tests/AppTest.php::AppTest.testRun",
        "tests/AppTest.php",
        13,
    ),
    (
        "CALLS",
        "@tests/AppTest.php::AppTest.testRun",
        "Demo\\App",
        "tests/AppTest.php",
        15,
    ),
    (
        "TESTED_BY",
        "Demo\\App",
        "@tests/AppTest.php::AppTest.testRun",
        "tests/AppTest.php",
        15,
    ),
    (
        "CALLS",
        "@tests/AppTest.php::AppTest.testRun",
        "@Demo/App.php::App.run",
        "tests/AppTest.php",
        16,
    ),
    (
        "CALLS",
        "@tests/AppTest.php::AppTest.testRun",
        "@Demo/Helper.php::Helper",
        "tests/AppTest.php",
        16,
    ),
    (
        "TESTED_BY",
        "@Demo/App.php::App.run",
        "@tests/AppTest.php::AppTest.testRun",
        "tests/AppTest.php",
        16,
    ),
    (
        "TESTED_BY",
        "@Demo/Helper.php::Helper",
        "@tests/AppTest.php::AppTest.testRun",
        "tests/AppTest.php",
        16,
    ),
    (
        "CONTAINS",
        "@tests/AppTest.php::AppTest",
        "@tests/AppTest.php::AppTest.buildsFresh",
        "tests/AppTest.php",
        20,
    ),
    (
        "CALLS",
        "@tests/AppTest.php::AppTest.buildsFresh",
        "@Demo/App.php::App",
        "tests/AppTest.php",
        22,
    ),
    (
        "TESTED_BY",
        "@Demo/App.php::App",
        "@tests/AppTest.php::AppTest.buildsFresh",
        "tests/AppTest.php",
        22,
    ),
    (
        "CALLS",
        "@tests/AppTest.php::AppTest.buildsFresh",
        "@Demo/App.php::App.build",
        "tests/AppTest.php",
        23,
    ),
    (
        "TESTED_BY",
        "@Demo/App.php::App.build",
        "@tests/AppTest.php::AppTest.buildsFresh",
        "tests/AppTest.php",
        23,
    ),
    (
        "CONTAINS",
        "@tests/AppTest.php::AppTest",
        "@tests/AppTest.php::AppTest.helperValue",
        "tests/AppTest.php",
        26,
    ),
    (
        "CALLS",
        "@tests/AppTest.php::AppTest.helperValue",
        "@Demo/Helper.php::Helper.make",
        "tests/AppTest.php",
        28,
    ),
    (
        "CALLS",
        "@tests/AppTest.php::AppTest.helperValue",
        "@Demo/Helper.php::Helper.twice",
        "tests/AppTest.php",
        28,
    ),
];

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

/// The probe graph rooted at `root`, with node and edge rows (ids) in permuted order.
fn probe_graph(root: &str, seed: u64) -> AnalysisGraph {
    let abs = |s: &str| match s.strip_prefix('@') {
        Some(rel) => format!("{root}{rel}"),
        None => s.to_string(),
    };
    let nodes: Vec<AnalysisNode> = permutation(NODES.len(), seed)
        .into_iter()
        .enumerate()
        .map(|(row, i)| {
            let (kind, qn, start, end, is_test) = NODES[i];
            let file = qn.split("::").next().unwrap();
            let name = qn.rsplit("::").next().unwrap();
            let name = name.rsplit('.').next().unwrap();
            AnalysisNode {
                id: row as i64 + 1,
                qualified: format!("{root}{qn}"),
                name: if kind == "File" {
                    format!("{root}{qn}")
                } else {
                    name.to_string()
                },
                kind: kind.to_string(),
                file_path: format!("{root}{file}"),
                language: Some("php".to_string()),
                line_start: Some(start),
                line_end: Some(end),
                is_test,
                decorators: Vec::new(),
                community_id: None,
                is_virtual: false,
                parent_name: None,
            }
        })
        .collect();
    let edges: Vec<AnalysisEdge> = permutation(EDGES.len(), seed.wrapping_mul(31))
        .into_iter()
        .map(|i| {
            let (kind, source, target, file, line) = EDGES[i];
            AnalysisEdge {
                kind: kind.to_string(),
                source: abs(source),
                target: abs(target),
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

/// Communities in output order, members made repo-relative and sorted.
fn detected(root: &str, seed: u64) -> Vec<(String, i64, Vec<String>)> {
    detect_communities(&probe_graph(root, seed), 2)
        .into_iter()
        .map(|c| {
            let mut members: Vec<String> = c
                .members
                .iter()
                .map(|m| m.strip_prefix(root).unwrap_or(m).to_string())
                .collect();
            members.sort();
            (c.name, c.level, members)
        })
        .collect()
}

#[test]
fn communities_do_not_depend_on_root_or_row_order() {
    let reference = detected("/w/a/", 0);
    assert!(!reference.is_empty());
    for (root, seed) in [
        ("/w/a/", 1),
        ("/w/a2/", 0),
        ("/tmp/a/much/longer/work/dir/", 7),
        ("/w/b/", 42),
        ("/w/c/", 1234),
        ("/w/d/", 99991),
    ] {
        assert_eq!(
            detected(root, seed),
            reference,
            "root {root} seed {seed} changed the communities"
        );
    }
}
