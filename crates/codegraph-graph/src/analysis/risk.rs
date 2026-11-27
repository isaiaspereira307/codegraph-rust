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
/// Lifecycle names only in a migration file (`_MIGRATION_METHOD_NAMES`).
const MIGRATION_METHOD_NAMES: &[&str] = &["up", "down"];
/// Folders that make a file a migration (`_MIGRATION_DIRS`): Laravel/Phinx/Doctrine/Knex, Rails.
const MIGRATION_DIRS: &[&str] = &["migrations", "migrate"];

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
/// node belongs to, in flow id order; `flow_count` is its `flow_memberships` rows, which only
/// score (0.05 each) when no membership has a flow row to give a criticality; `churn_commits`
/// is the commits to its file in the churn window, or `None` when churn is off or unavailable.
pub fn risk_signals(
    g: &AnalysisGraph,
    ix: usize,
    flow_criticalities: &[f64],
    flow_count: usize,
    churn_commits: Option<u32>,
) -> RiskSignals {
    let node = &g.nodes[ix];

    // A flow with criticality 0.0 is still a membership with a criticality.
    let flows = if flow_criticalities.is_empty() {
        (flow_count as f64 * 0.05).min(0.25)
    } else {
        flow_criticalities.iter().sum::<f64>().min(0.25)
    };

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

/// A lifecycle method (`setUp`, `__init__`, ..., or `up`/`down` of a migration) of test code
/// (`_is_test_lifecycle`). The CRG also accepts a node in a test file that is not marked
/// `is_test`; the graph crate has no test-path rules, so only `is_test` counts here.
fn is_test_lifecycle(g: &AnalysisGraph, ix: usize) -> bool {
    let n = &g.nodes[ix];
    let name = n.name.as_str();
    n.kind == "Function"
        && n.is_test
        && (TEST_LIFECYCLE_NAMES.contains(&name)
            || (MIGRATION_METHOD_NAMES.contains(&name) && is_migration_file(&n.file_path)))
}

/// CRG `_is_migration_file` as risk scoring calls it, with no repository root: a relative
/// path with a `migrations`/`migrate` folder. An absolute path cannot be placed in the
/// repository without the root, so it never is one.
fn is_migration_file(file_path: &str) -> bool {
    let path = file_path.replace('\\', "/");
    let absolute = path.starts_with('/')
        || (path.as_bytes().get(1) == Some(&b':') && path.as_bytes()[0].is_ascii_alphabetic());
    if absolute {
        return false;
    }
    let parts: Vec<&str> = path.split('/').collect();
    parts[..parts.len().saturating_sub(1)]
        .iter()
        .any(|part| MIGRATION_DIRS.contains(part))
}

/// Distinct test nodes covering node `ix` (`get_transitive_tests` with `max_depth = 1` and no
/// helper routes): TESTED_BY from the node (a Class adds its CONTAINS members, a File its
/// classes and functions), the bare-name fallback (`bare_name_tests`), then TESTED_BY from its
/// bound CALLS callees, at most 50 of them,
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
    for qn in &inputs {
        collect_tested_by(g, qn, &mut seen, &mut count);
    }
    bare_name_tests(g, node, &mut seen, &mut count);

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
            collect_tested_by(g, callee, &mut seen, &mut count);
        }
    }
    count
}

/// Counts the resolved TESTED_BY targets of `source` not seen yet that have a node.
fn collect_tested_by<'g>(
    g: &'g AnalysisGraph,
    source: &str,
    seen: &mut BTreeSet<&'g str>,
    count: &mut usize,
) {
    for e in g.edges_from(source) {
        if e.kind != "TESTED_BY" || e.unresolved {
            continue;
        }
        if seen.insert(e.target.as_str()) && g.index_of(&e.target).is_some() {
            *count += 1;
        }
    }
}

/// CRG `get_transitive_tests` bare-name fallback for TESTED_BY edges whose source is the
/// node's bare name (`qualified.rsplit("::")`, so a method keeps its class): the edge counts
/// when the test file itself or a file it imports holds exactly one Function/Test/Class of that
/// name, that one is this node, and the edge's receiver evidence can mean it.
fn bare_tested_by_admits(
    g: &AnalysisGraph,
    node: &crate::analysis::AnalysisNode,
    bare: &str,
    edge: &crate::analysis::AnalysisEdge,
    imports: &mut std::collections::HashMap<String, BTreeSet<String>>,
) -> bool {
    let context = edge.file_path.as_str();
    let imported = imports.entry(context.to_string()).or_insert_with(|| {
        let modules = g.python_module_files();
        let mut files = BTreeSet::new();
        for import in g.imports_in(context) {
            let target = import.target.split("::").next().unwrap_or_default();
            files.insert(target.to_string());
            if let Some(file) = modules.get(target) {
                files.insert(file.clone());
            }
        }
        files
    });
    let mut supported = g
        .nodes_named(bare)
        .filter(|c| matches!(c.kind.as_str(), "Function" | "Test" | "Class"))
        .filter(|c| c.file_path == context || imported.contains(&c.file_path));
    let (Some(only), None) = (supported.next(), supported.next()) else {
        return false;
    };
    only.qualified == node.qualified && receiver_evidence_admits(edge, node)
}

fn bare_name_tests<'g>(
    g: &'g AnalysisGraph,
    node: &crate::analysis::AnalysisNode,
    seen: &mut BTreeSet<&'g str>,
    count: &mut usize,
) {
    let bare = node
        .qualified
        .rsplit_once("::")
        .map_or(node.qualified.as_str(), |(_, b)| b);
    let mut imports = std::collections::HashMap::new();
    for e in g.edges_from(bare) {
        if e.kind != "TESTED_BY" || e.unresolved {
            continue;
        }
        if !bare_tested_by_admits(g, node, bare, e, &mut imports) {
            continue;
        }
        if seen.insert(e.target.as_str()) && g.index_of(&e.target).is_some() {
            *count += 1;
        }
    }
}

/// CRG `_receiver_evidence_admits_candidate` over the single candidate `node`.
fn receiver_evidence_admits(
    edge: &crate::analysis::AnalysisEdge,
    node: &crate::analysis::AnalysisNode,
) -> bool {
    let Some(extra) = edge.evidence.as_deref() else {
        return true;
    };
    let text = |key: &str| extra.get(key).and_then(|v| v.as_str());
    let parent = node.parent_name.as_deref().filter(|p| !p.is_empty());
    match extra.get("receiver_binding") {
        None => {
            // `_claimed_owner`: `a.py::Helper.ping` names `Helper`.
            let claimed = text("claimed_endpoint").and_then(|c| {
                let leaf = c.rsplit("::").next().unwrap_or(c);
                leaf.rsplit_once('.')
                    .map(|(owner, _)| owner)
                    .filter(|o| !o.is_empty())
            });
            let owner = claimed.or_else(|| {
                (text("receiver_resolution") == Some("typed_receiver_unresolved"))
                    .then(|| text("receiver_type"))
                    .flatten()
            });
            owner.is_none_or(|owner| same_owner(parent, owner))
        }
        Some(binding) => match binding.as_str() {
            Some("module") => text("receiver_module")
                .filter(|m| !m.is_empty())
                .is_some_and(|m| node.file_path == m && parent.is_none()),
            Some("class") => {
                let Some(class) = text("receiver_class").filter(|c| !c.is_empty()) else {
                    return false;
                };
                let class_file = match extra.get("receiver_class_file") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(serde_json::Value::String(f)) => Some(f.as_str()),
                    Some(_) => return false,
                };
                let scopes: Vec<String> = extra
                    .get("receiver_scopes")
                    .and_then(|s| s.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|s| s.as_str().filter(|s| !s.is_empty()))
                    .map(|scope| format!("{scope}.{class}"))
                    .chain(std::iter::once(class.to_string()))
                    .collect();
                // The first scope path any candidate answers to decides; here the only
                // candidate is the node, so it is admitted when it answers to one of them.
                scopes.iter().any(|path| {
                    parent == Some(path.as_str())
                        && class_file.is_none_or(|f| f.is_empty() || node.file_path == f)
                })
            }
            _ => false,
        },
    }
}

/// CRG `_same_owner`: scopes compared from the innermost name out, as far as both go.
fn same_owner(parent: Option<&str>, owner: &str) -> bool {
    let have: Vec<&str> = parent.unwrap_or("").split('.').collect();
    let want: Vec<&str> = owner.split('.').collect();
    let depth = have.len().min(want.len());
    have[have.len() - depth..] == want[want.len() - depth..]
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
            parent_name: None,
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
            evidence: None,
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
        let s = risk_signals(&g, ix(&g, "a.py::check_password"), &[], 0, None);
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
        assert_eq!(risk_signals(&g, f, &[0.1, 0.05], 0, None).flows, 0.1 + 0.05);
        assert_eq!(risk_signals(&g, f, &[0.2, 0.2, 0.2], 0, None).flows, 0.25);
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
        let s = risk_signals(&g, ix(&g, "a.py::f"), &[], 0, None);
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
        let s = risk_signals(&g, ix(&g, "a.py::f"), &[], 0, None);
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
        let s = risk_signals(&g, ix(&g, "a.py::f"), &[], 0, None);
        // 4 distinct tests: 0.30 - 0.8 * 0.25 = 0.10.
        assert!((s.tests - 0.10).abs() < 1e-12, "{}", s.tests);

        nodes.push(node(14, "Test", "t.py::test_4", None));
        edges.push(edge("TESTED_BY", "a.py::f", "t.py::test_4"));
        let g = graph(nodes, edges);
        let s = risk_signals(&g, ix(&g, "a.py::f"), &[], 0, None);
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
        let s = risk_signals(&g, ix(&g, "a.py::Auth"), &[], 0, None);
        assert!((s.tests - 0.25).abs() < 1e-12, "{}", s.tests);
    }

    #[test]
    fn test_lifecycle_method_has_no_coverage_term() {
        let mut setup = node(1, "Function", "t.py::setUp", None);
        setup.is_test = true;
        let g = graph(vec![setup], vec![]);
        assert_eq!(risk_signals(&g, 0, &[], 0, None).tests, 0.0);
    }

    /// CRG `_risk_signals_from`: memberships with no flow row (no criticality) still score
    /// 0.05 each, capped at 0.25; criticalities win when there are any.
    #[test]
    fn flow_count_fallback_without_criticalities() {
        let g = graph(vec![node(1, "Function", "a.py::f", None)], vec![]);
        let f = ix(&g, "a.py::f");
        assert!((risk_signals(&g, f, &[], 3, None).flows - 0.15).abs() < 1e-12);
        assert_eq!(risk_signals(&g, f, &[], 9, None).flows, 0.25);
        assert_eq!(risk_signals(&g, f, &[0.1], 3, None).flows, 0.1);
    }

    /// CRG `is_test_gap_exempt` without a repository root: `up`/`down` are lifecycle names
    /// only in a relative `migrations`/`migrate` folder; an absolute path is never placed.
    #[test]
    fn migration_up_and_down_have_no_coverage_term() {
        let tests_of = |qn: &str| {
            let mut n = node(1, "Function", qn, None);
            n.is_test = true;
            let g = graph(vec![n], vec![]);
            risk_signals(&g, 0, &[], 0, None).tests
        };
        assert_eq!(tests_of("db/migrations/001_init.py::up"), 0.0);
        assert_eq!(tests_of("db/migrate/002.rb::down"), 0.0);
        assert_eq!(tests_of("src/app.py::up"), 0.30);
        assert_eq!(tests_of("migrations.py::up"), 0.30);
        assert_eq!(tests_of("/r/db/migrations/001_init.py::up"), 0.30);
        assert_eq!(tests_of("db/migrations/001_init.py::apply"), 0.30);
    }

    /// CRG `get_transitive_tests` bare-name fallback: a TESTED_BY edge whose source is the
    /// bare name counts when the test file holds or imports exactly one candidate of that name,
    /// and its receiver evidence can mean the node.
    #[test]
    fn bare_tested_by_counts_with_import_evidence() {
        let tested = |extra_nodes: Vec<AnalysisNode>, evidence: Option<serde_json::Value>| {
            let mut nodes = vec![
                node(1, "Function", "a.py::f", None),
                node(2, "Test", "t.py::test_f", None),
                node(3, "File", "a.py", None),
            ];
            nodes.extend(extra_nodes);
            let edges = vec![
                AnalysisEdge {
                    file_path: "t.py".into(),
                    evidence: evidence.and_then(|v| v.as_object().cloned()).map(Box::new),
                    ..edge("TESTED_BY", "f", "t.py::test_f")
                },
                AnalysisEdge {
                    file_path: "t.py".into(),
                    ..edge("IMPORTS_FROM", "t.py", "a.py")
                },
                AnalysisEdge {
                    file_path: "t.py".into(),
                    ..edge("IMPORTS_FROM", "t.py", "b.py")
                },
            ];
            let g = graph(nodes, edges);
            risk_signals(&g, ix(&g, "a.py::f"), &[], 0, None).tests
        };
        assert_eq!(tested(vec![], None), 0.25);
        // A second imported candidate makes the name ambiguous.
        assert_eq!(
            tested(vec![node(4, "Function", "b.py::f", None)], None),
            0.30
        );
        // A candidate the test file does not import is no evidence against.
        assert_eq!(
            tested(vec![node(4, "Function", "c.py::f", None)], None),
            0.25
        );
        // Receiver evidence naming a module that is not a.py rules it out.
        let module = serde_json::json!({"receiver_binding": "module", "receiver_module": "z.py"});
        assert_eq!(tested(vec![], Some(module)), 0.30);
        let module = serde_json::json!({"receiver_binding": "module", "receiver_module": "a.py"});
        assert_eq!(tested(vec![], Some(module)), 0.25);
        // A local with no evidence (`some_dict.f()`) admits nothing.
        let local = serde_json::json!({"receiver_binding": "local"});
        assert_eq!(tested(vec![], Some(local)), 0.30);
        // A typed receiver the resolver could not place must own the method.
        let typed = serde_json::json!({
            "receiver_resolution": "typed_receiver_unresolved", "receiver_type": "Pet"
        });
        assert_eq!(tested(vec![], Some(typed)), 0.30);
    }

    /// Only the node's own bare name falls back, and a method's bare name keeps its class
    /// (`Auth.run`), which no node is named, as in the CRG.
    #[test]
    fn bare_tested_by_does_not_reach_methods() {
        let mut run = node(1, "Function", "a.py::Auth.run", None);
        run.name = "run".into();
        run.parent_name = Some("Auth".into());
        let g = graph(
            vec![run, node(2, "Test", "t.py::test_run", None)],
            vec![
                AnalysisEdge {
                    file_path: "a.py".into(),
                    ..edge("TESTED_BY", "Auth.run", "t.py::test_run")
                },
                AnalysisEdge {
                    file_path: "a.py".into(),
                    ..edge("TESTED_BY", "run", "t.py::test_run")
                },
            ],
        );
        assert_eq!(risk_signals(&g, 0, &[], 0, None).tests, 0.30);
    }

    #[test]
    fn churn_saturates() {
        let g = graph(vec![node(1, "Function", "a.py::f", None)], vec![]);
        let f = ix(&g, "a.py::f");
        assert_eq!(risk_signals(&g, f, &[], 0, None).churn, 0.0);
        assert_eq!(risk_signals(&g, f, &[], 0, Some(0)).churn, 0.0);
        assert!((risk_signals(&g, f, &[], 0, Some(4)).churn - 0.06).abs() < 1e-12);
        assert_eq!(risk_signals(&g, f, &[], 0, Some(10)).churn, 0.15);
        assert_eq!(risk_signals(&g, f, &[], 0, Some(250)).churn, 0.15);
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
