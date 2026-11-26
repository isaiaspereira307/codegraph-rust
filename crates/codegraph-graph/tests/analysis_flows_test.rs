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

use codegraph_graph::analysis::flows::{FLOW_MAX_DEPTH, FlowWriteStats, store_flows, trace_flows};

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

// ---- GraphQuery flow reads ----

use codegraph_graph::analysis::{PostprocessLevel, run_postprocess};
use codegraph_graph::graph_query::GraphQuery;

/// Two flows: `main` (3 nodes, two files) and `zeta_job` (2 nodes); both rooted in functions.
fn flow_db() -> (tempfile::TempDir, GraphQuery) {
    let (dir, store) = fixture_db(
        &[
            ("Function", "a.py::main", "main", "a.py", false),
            ("Function", "a.py::helper", "helper", "a.py", false),
            ("Function", "b.py::leaf", "leaf", "b.py", false),
            ("Function", "c.py::zeta_job", "zeta_job", "c.py", false),
            ("Function", "c.py::step", "step", "c.py", false),
            ("Class", "d.py::Svc", "Svc", "d.py", false),
        ],
        &[
            ("CALLS", "a.py::main", "a.py::helper"),
            ("CALLS", "a.py::helper", "b.py::leaf"),
            ("CALLS", "c.py::zeta_job", "c.py::step"),
        ],
    );
    let mut conn = open_conn(dir.path());
    run_postprocess(&mut conn, PostprocessLevel::Full).unwrap();
    (dir, GraphQuery::new(store))
}

#[tokio::test]
async fn list_flows_sorts_and_caps() {
    let (_d, q) = flow_db();
    let (flows, total) = q.list_flows("criticality", 50, None).await.unwrap();
    assert_eq!((flows.len(), total), (2, 2));
    assert!(flows[0].criticality >= flows[1].criticality);
    let (by_name, _) = q.list_flows("name", 50, None).await.unwrap();
    assert_eq!(by_name[0].name, "main");
    let (by_nodes, _) = q.list_flows("node_count", 1, None).await.unwrap();
    assert_eq!((by_nodes.len(), by_nodes[0].node_count), (1, 3));
    let (fallback, _) = q.list_flows("bogus", 50, None).await.unwrap();
    assert_eq!(fallback[0].id, flows[0].id);
}

#[tokio::test]
async fn list_flows_filters_by_entry_kind() {
    let (_d, q) = flow_db();
    let (none, total) = q
        .list_flows("criticality", 50, Some("Class"))
        .await
        .unwrap();
    assert!(none.is_empty());
    assert_eq!(total, 0);
    let (some, total) = q
        .list_flows("criticality", 50, Some("Function"))
        .await
        .unwrap();
    assert_eq!((some.len(), total), (2, 2));
}

#[tokio::test]
async fn get_flow_by_name_substring_case_insensitive() {
    let (_d, q) = flow_db();
    let flow = q.get_flow(None, Some("ZETA"), 50).await.unwrap().unwrap();
    assert_eq!(flow.flow.name, "zeta_job");
    assert_eq!(flow.steps.len(), 2);
    assert_eq!(flow.steps[0].name, "zeta_job");
    assert_eq!((flow.total_steps, flow.truncated), (2, false));
    let capped = q
        .get_flow(Some(flow.flow.id), None, 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((capped.steps.len(), capped.truncated), (1, true));
}

#[tokio::test]
async fn get_flow_missing_is_none() {
    let (_d, q) = flow_db();
    assert!(q.get_flow(None, Some("nope"), 50).await.unwrap().is_none());
    assert!(q.get_flow(Some(9999), None, 50).await.unwrap().is_none());
}

// ---- stable flow IDs across post-process runs ----

/// (entry qualified_name, flow id, updated_at, path_json), ordered by entry.
fn stored_flows(conn: &rusqlite::Connection) -> Vec<(String, i64, String, String)> {
    conn.prepare(
        "SELECT n.qualified_name, f.id, f.updated_at, f.path_json FROM flows f \
         JOIN nodes n ON n.id = f.entry_point_id ORDER BY n.qualified_name",
    )
    .unwrap()
    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

/// (flow_id, node_id, position) for every membership row.
fn memberships(conn: &rusqlite::Connection) -> Vec<(i64, i64, i64)> {
    conn.prepare("SELECT flow_id, node_id, position FROM flow_memberships ORDER BY 1, 3")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn add_call(conn: &rusqlite::Connection, src: &str, tgt: &str) {
    conn.execute(
        "INSERT INTO edges (kind, source_qualified, target_qualified, file_path, line, \
         updated_at) VALUES ('CALLS', ?1, ?2, '', 1, 0)",
        [src, tgt],
    )
    .unwrap();
}

#[test]
fn flow_ids_stable_across_unchanged_postprocess_runs() {
    let (dir, _q) = flow_db();
    let mut conn = open_conn(dir.path());
    let first = stored_flows(&conn);
    assert_eq!(first.len(), 2);
    let first_members = memberships(&conn);
    // A sentinel shows whether the second run rewrote rows it did not need to.
    conn.execute("UPDATE flows SET updated_at = 'sentinel'", [])
        .unwrap();
    let stats = run_postprocess(&mut conn, PostprocessLevel::Full).unwrap();
    assert_eq!(
        stats.flow_writes,
        FlowWriteStats {
            inserted: 0,
            updated: 0,
            deleted: 0,
            unchanged: 2
        }
    );
    assert_eq!(stats.flows, 2);
    let second = stored_flows(&conn);
    let ids = |v: &[(String, i64, String, String)]| {
        v.iter()
            .map(|(q, id, _, _)| (q.clone(), *id))
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&second), ids(&first));
    assert!(second.iter().all(|(_, _, u, _)| u == "sentinel"));
    assert_eq!(memberships(&conn), first_members);
}

#[test]
fn flow_id_survives_member_change() {
    let (dir, _q) = flow_db();
    let mut conn = open_conn(dir.path());
    let first = stored_flows(&conn);
    conn.execute("UPDATE flows SET updated_at = 'sentinel'", [])
        .unwrap();
    // main gains a member: main -> helper -> leaf, main -> c.py::step.
    add_call(&conn, "a.py::main", "c.py::step");
    let stats = run_postprocess(&mut conn, PostprocessLevel::Full).unwrap();
    assert_eq!(
        stats.flow_writes,
        FlowWriteStats {
            inserted: 0,
            updated: 1,
            deleted: 0,
            unchanged: 1
        }
    );
    let second = stored_flows(&conn);
    let (main_before, main_after) = (&first[0], &second[0]);
    assert_eq!(main_after.0, "a.py::main");
    assert_eq!(main_after.1, main_before.1, "same entry point keeps its id");
    assert_ne!(main_after.2, "sentinel", "changed flow is rewritten");
    assert_ne!(main_after.3, main_before.3);
    let (node_count,): (i64,) = conn
        .query_row(
            "SELECT node_count FROM flows WHERE id = ?1",
            [main_after.1],
            |r| Ok((r.get(0)?,)),
        )
        .unwrap();
    assert_eq!(node_count, 4);
    let main_members: Vec<i64> = memberships(&conn)
        .into_iter()
        .filter(|(f, _, _)| *f == main_after.1)
        .map(|(_, n, _)| n)
        .collect();
    assert_eq!(main_members.len(), 4);
    // The untouched flow is left alone.
    assert_eq!(second[1].1, first[1].1);
    assert_eq!(second[1].2, "sentinel");
}

#[test]
fn removed_entry_point_deletes_flow_and_memberships() {
    let (dir, _q) = flow_db();
    let mut conn = open_conn(dir.path());
    let first = stored_flows(&conn);
    let zeta_id = first[1].1;
    assert_eq!(first[1].0, "c.py::zeta_job");
    conn.execute(
        "DELETE FROM nodes WHERE qualified_name = 'c.py::zeta_job'",
        [],
    )
    .unwrap();
    conn.execute(
        "DELETE FROM edges WHERE source_qualified = 'c.py::zeta_job'",
        [],
    )
    .unwrap();
    let stats = run_postprocess(&mut conn, PostprocessLevel::Full).unwrap();
    assert_eq!(
        stats.flow_writes,
        FlowWriteStats {
            inserted: 0,
            updated: 0,
            deleted: 1,
            unchanged: 1
        }
    );
    assert_eq!(stats.flows, 1);
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM flows")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(ids, vec![first[0].1]);
    assert!(memberships(&conn).iter().all(|(f, _, _)| *f != zeta_id));
    assert_eq!(memberships(&conn).len(), 3);
}

#[test]
fn store_flows_fresh_ids_follow_criticality_order() {
    let g = flow_graph(
        &["main", "a", "zeta_job", "b"],
        &[("main", "a"), ("zeta_job", "b")],
        &[],
    );
    let flows = trace_flows(&g, FLOW_MAX_DEPTH);
    let (dir, _s) = fixture_db(&[], &[]);
    let mut conn = open_conn(dir.path());
    let tx = conn.transaction().unwrap();
    assert_eq!(store_flows(&tx, &flows).unwrap(), 2);
    tx.commit().unwrap();
    let names: Vec<String> = conn
        .prepare("SELECT name FROM flows ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let expected: Vec<String> = flows.iter().map(|f| f.name.clone()).collect();
    assert_eq!(names, expected);
}

#[tokio::test]
async fn affected_flows_from_file() {
    let (_d, q) = flow_db();
    let hit = q.affected_flows(&["b.py".to_string()]).await.unwrap();
    assert_eq!(hit.len(), 1);
    assert_eq!(hit[0].flow.name, "main");
    let both = q
        .affected_flows(&["b.py".to_string(), "c.py".to_string()])
        .await
        .unwrap();
    assert_eq!(both.len(), 2);
    assert!(
        q.affected_flows(&["zzz.py".to_string()])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(q.affected_flows(&[]).await.unwrap().is_empty());
}
