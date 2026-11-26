// ABOUTME: CRG review risk score (changes.py risk_signals): flows, communities, tests, security,
// ABOUTME: callers and churn terms for one node, summed and clamped to 0.0 - 1.0.

use super::{AnalysisGraph, SECURITY_KEYWORDS, round_half_even};
use serde::Serialize;
use std::collections::BTreeSet;

/// Callees kept per hop of the transitive test walk (`CRG_MAX_TRANSITIVE_FRONTIER`).
const MAX_TEST_FRONTIER: usize = 50;
/// Commits at which the churn term saturates, and its weight.
const CHURN_SATURATION: f64 = 10.0;
const CHURN_WEIGHT: f64 = 0.15;
/// Test lifecycle names that carry no coverage to lack (`_TEST_GAP_EXEMPT_NAMES`).
const TEST_LIFECYCLE_NAMES: &[&str] = &[
    "setUp",
    "tearDown",
    "setUpBeforeClass",
    "tearDownAfterClass",
    "setup_method",
    "teardown_method",
    "setUpClass",
    "tearDownClass",
    "__construct",
    "__init__",
    "__destruct",
];

/// Each term of the risk score, in the order the score adds them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct RiskSignals {
    pub flows: f64,
    pub cross_community: f64,
    pub tests: f64,
    pub security: f64,
    pub callers: f64,
    pub churn: f64,
}

/// The risk terms of node `ix`. `flow_criticalities` holds the criticality of every flow the
/// node belongs to, in flow id order; `churn_commits` is the commits to its file in the churn
/// window, or `None` when churn is off or unavailable.
pub fn risk_signals(
    g: &AnalysisGraph,
    ix: usize,
    flow_criticalities: &[f64],
    churn_commits: Option<u32>,
) -> RiskSignals {
    let node = &g.nodes[ix];

    // A flow with criticality 0.0 is still a membership; only membership without any flow row
    // would fall back to 0.05 per flow, and the caller has no such rows to give.
    let flows = flow_criticalities.iter().sum::<f64>().min(0.25);

    let callers: Vec<&str> = g
        .edges_to(&node.qualified)
        .filter(|e| e.kind == "CALLS")
        .map(|e| e.source.as_str())
        .collect();
    let crossing = match node.community_id {
        None => 0,
        Some(own) => callers
            .iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|qn| {
                g.index_of(qn)
                    .and_then(|i| g.nodes[i].community_id)
                    .is_some_and(|cid| cid != own)
            })
            .count(),
    };
    let cross_community = (crossing as f64 * 0.05).min(0.15);

    let tests = if is_test_lifecycle(g, ix) {
        0.0
    } else {
        let count = transitive_test_count(g, ix) as f64;
        0.30 - (count / 5.0).min(1.0) * 0.25
    };

    let name = node.name.to_lowercase();
    let qualified = node.qualified.to_lowercase();
    let sensitive = SECURITY_KEYWORDS
        .iter()
        .any(|kw| name.contains(kw) || qualified.contains(kw));
    let security = if sensitive { 0.20 } else { 0.0 };

    let callers = (callers.len() as f64 / 20.0).min(0.10);

    let churn = match churn_commits {
        Some(c) if !node.file_path.is_empty() => {
            (c as f64 / CHURN_SATURATION).min(1.0) * CHURN_WEIGHT
        }
        _ => 0.0,
    };

    RiskSignals {
        flows,
        cross_community,
        tests,
        security,
        callers,
        churn,
    }
}

/// The signals summed in order, clamped to 0.0 - 1.0 and rounded to 4 places.
pub fn risk_score(s: &RiskSignals) -> f64 {
    let sum = s.flows + s.cross_community + s.tests + s.security + s.callers + s.churn;
    round_half_even(sum.clamp(0.0, 1.0), 4)
}

/// A lifecycle method (`setUp`, `__init__`, ...) of test code.
fn is_test_lifecycle(g: &AnalysisGraph, ix: usize) -> bool {
    let n = &g.nodes[ix];
    n.kind == "Function" && n.is_test && TEST_LIFECYCLE_NAMES.contains(&n.name.as_str())
}

/// Distinct test nodes covering node `ix` (`get_transitive_tests` with `max_depth = 1` and no
/// helper routes): TESTED_BY from the node (a Class adds its CONTAINS members, a File its
/// classes and functions), then TESTED_BY from its bound CALLS callees, at most 50 of them,
/// bound callees first and by name. Unresolved edges are skipped; tests without a node are
/// not counted.
fn transitive_test_count(g: &AnalysisGraph, ix: usize) -> usize {
    let node = &g.nodes[ix];
    let mut inputs = vec![node.qualified.as_str()];
    match node.kind.as_str() {
        "Class" => inputs.extend(
            g.edges_from(&node.qualified)
                .filter(|e| e.kind == "CONTAINS")
                .map(|e| e.target.as_str()),
        ),
        "File" => inputs.extend(
            g.nodes
                .iter()
                .filter(|n| {
                    n.file_path == node.file_path
                        && n.qualified != node.qualified
                        && matches!(n.kind.as_str(), "Class" | "Function" | "Method")
                })
                .map(|n| n.qualified.as_str()),
        ),
        _ => {}
    }

    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut count = 0;
    let mut collect = |source: &str, count: &mut usize| {
        for e in g.edges_from(source) {
            if e.kind != "TESTED_BY" || e.unresolved {
                continue;
            }
            if seen.insert(e.target.as_str()) && g.index_of(&e.target).is_some() {
                *count += 1;
            }
        }
    };
    for qn in &inputs {
        collect(qn, &mut count);
    }

    let frontier: BTreeSet<&str> = inputs.iter().copied().collect();
    let callees: BTreeSet<&str> = frontier
        .iter()
        .flat_map(|qn| g.edges_from(qn))
        .filter(|e| e.kind == "CALLS" && !e.unresolved)
        .map(|e| e.target.as_str())
        .collect();
    let mut callees: Vec<&str> = callees.into_iter().collect();
    callees.sort_by_key(|qn| (g.index_of(qn).is_none(), *qn));
    for callee in callees.into_iter().take(MAX_TEST_FRONTIER) {
        if g.index_of(callee).is_some() {
            collect(callee, &mut count);
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{AnalysisEdge, AnalysisNode};

    fn node(id: i64, kind: &str, qn: &str, community_id: Option<i64>) -> AnalysisNode {
        AnalysisNode {
            id,
            qualified: qn.into(),
            name: qn.rsplit("::").next().unwrap().into(),
            kind: kind.into(),
            file_path: qn.split("::").next().unwrap().into(),
            language: None,
            line_start: None,
            line_end: None,
            is_test: kind == "Test",
            decorators: vec![],
            community_id,
            is_virtual: false,
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
            unresolved: false,
        }
    }

    fn graph(nodes: Vec<AnalysisNode>, edges: Vec<AnalysisEdge>) -> AnalysisGraph {
        AnalysisGraph::from_parts(nodes, edges)
    }

    fn ix(g: &AnalysisGraph, qn: &str) -> usize {
        g.index_of(qn).unwrap()
    }

    #[test]
    fn untested_security_function_scores_high() {
        let g = graph(
            vec![node(1, "Function", "a.py::check_password", None)],
            vec![],
        );
        let s = risk_signals(&g, ix(&g, "a.py::check_password"), &[], None);
        assert_eq!(s.tests, 0.30);
        assert_eq!(s.security, 0.20);
        assert_eq!(
            (s.flows, s.cross_community, s.callers, s.churn),
            (0.0, 0.0, 0.0, 0.0)
        );
        assert_eq!(risk_score(&s), 0.5);
    }

    #[test]
    fn flow_signal_caps_at_quarter() {
        let g = graph(vec![node(1, "Function", "a.py::f", None)], vec![]);
        let f = ix(&g, "a.py::f");
        assert_eq!(risk_signals(&g, f, &[0.1, 0.05], None).flows, 0.1 + 0.05);
        assert_eq!(risk_signals(&g, f, &[0.2, 0.2, 0.2], None).flows, 0.25);
    }

    #[test]
    fn cross_community_callers() {
        let mut nodes = vec![node(1, "Function", "a.py::f", Some(1))];
        let mut edges = Vec::new();
        // Four callers in community 2 (one calls twice), one in community 1, one without.
        for (i, cid) in [(2, Some(2)), (3, Some(2)), (4, Some(2)), (5, Some(2))] {
            nodes.push(node(i, "Function", &format!("b.py::c{i}"), cid));
            edges.push(edge("CALLS", &format!("b.py::c{i}"), "a.py::f"));
        }
        edges.push(edge("CALLS", "b.py::c2", "a.py::f"));
        nodes.push(node(6, "Function", "a.py::same", Some(1)));
        edges.push(edge("CALLS", "a.py::same", "a.py::f"));
        nodes.push(node(7, "Function", "c.py::loose", None));
        edges.push(edge("CALLS", "c.py::loose", "a.py::f"));
        let g = graph(nodes, edges);
        let s = risk_signals(&g, ix(&g, "a.py::f"), &[], None);
        // Four distinct cross-community callers: 0.20, capped at 0.15.
        assert_eq!(s.cross_community, 0.15);
        // Seven incoming CALLS edges: 7 / 20 = 0.35, capped at 0.10.
        assert_eq!(s.callers, 0.10);

        // A node without a community never crosses.
        let g = graph(
            vec![
                node(1, "Function", "a.py::f", None),
                node(2, "Function", "b.py::c", Some(2)),
            ],
            vec![edge("CALLS", "b.py::c", "a.py::f")],
        );
        let s = risk_signals(&g, ix(&g, "a.py::f"), &[], None);
        assert_eq!((s.cross_community, s.callers), (0.0, 0.05));
    }

    #[test]
    fn well_tested_low_risk() {
        let mut nodes = vec![
            node(1, "Function", "a.py::f", None),
            node(2, "Function", "a.py::g", None),
        ];
        let mut edges = vec![edge("CALLS", "a.py::f", "a.py::g")];
        // Three direct tests on f, two on its callee g (one shared), one unresolved.
        for i in 0..4 {
            nodes.push(node(10 + i, "Test", &format!("t.py::test_{i}"), None));
        }
        for i in 0..3 {
            edges.push(edge("TESTED_BY", "a.py::f", &format!("t.py::test_{i}")));
        }
        edges.push(edge("TESTED_BY", "a.py::g", "t.py::test_0"));
        edges.push(edge("TESTED_BY", "a.py::g", "t.py::test_3"));
        nodes.push(node(20, "Test", "t.py::test_guess", None));
        edges.push(AnalysisEdge {
            unresolved: true,
            ..edge("TESTED_BY", "a.py::f", "t.py::test_guess")
        });
        let g = graph(nodes.clone(), edges.clone());
        let s = risk_signals(&g, ix(&g, "a.py::f"), &[], None);
        // 4 distinct tests: 0.30 - 0.8 * 0.25 = 0.10.
        assert!((s.tests - 0.10).abs() < 1e-12, "{}", s.tests);

        nodes.push(node(14, "Test", "t.py::test_4", None));
        edges.push(edge("TESTED_BY", "a.py::f", "t.py::test_4"));
        let g = graph(nodes, edges);
        let s = risk_signals(&g, ix(&g, "a.py::f"), &[], None);
        assert!((s.tests - 0.05).abs() < 1e-12, "{}", s.tests);
        assert_eq!(risk_score(&s), 0.05);
    }

    #[test]
    fn class_counts_tests_of_its_members() {
        let g = graph(
            vec![
                node(1, "Class", "a.py::Auth", None),
                node(2, "Function", "a.py::Auth.run", None),
                node(3, "Test", "t.py::test_run", None),
            ],
            vec![
                edge("CONTAINS", "a.py::Auth", "a.py::Auth.run"),
                edge("TESTED_BY", "a.py::Auth.run", "t.py::test_run"),
            ],
        );
        let s = risk_signals(&g, ix(&g, "a.py::Auth"), &[], None);
        assert!((s.tests - 0.25).abs() < 1e-12, "{}", s.tests);
    }

    #[test]
    fn test_lifecycle_method_has_no_coverage_term() {
        let mut setup = node(1, "Function", "t.py::setUp", None);
        setup.is_test = true;
        let g = graph(vec![setup], vec![]);
        assert_eq!(risk_signals(&g, 0, &[], None).tests, 0.0);
    }

    #[test]
    fn churn_saturates() {
        let g = graph(vec![node(1, "Function", "a.py::f", None)], vec![]);
        let f = ix(&g, "a.py::f");
        assert_eq!(risk_signals(&g, f, &[], None).churn, 0.0);
        assert_eq!(risk_signals(&g, f, &[], Some(0)).churn, 0.0);
        assert!((risk_signals(&g, f, &[], Some(4)).churn - 0.06).abs() < 1e-12);
        assert_eq!(risk_signals(&g, f, &[], Some(10)).churn, 0.15);
        assert_eq!(risk_signals(&g, f, &[], Some(250)).churn, 0.15);
    }

    #[test]
    fn score_clamps_and_rounds() {
        let s = RiskSignals {
            flows: 0.25,
            cross_community: 0.15,
            tests: 0.30,
            security: 0.20,
            callers: 0.10,
            churn: 0.15,
        };
        assert_eq!(risk_score(&s), 1.0);
        let s = RiskSignals {
            tests: 0.1,
            churn: 0.06,
            callers: 0.05,
            ..Default::default()
        };
        assert_eq!(risk_score(&s), 0.21);
    }
}
