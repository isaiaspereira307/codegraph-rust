// ABOUTME: Tests the AnalysisGraph loader and the numeric/name helpers of the analysis module.
// ABOUTME: Fixtures are written by hand into SQLite, with no Rust-only cg_* rows.

#[path = "common/analysis_fixture.rs"]
mod analysis_fixture;

use analysis_fixture::{fixture_db, open_conn};
use codegraph_graph::analysis::{AnalysisGraph, round_half_even, sanitize_name};

#[test]
fn load_reads_nodes_and_edges_in_id_order() {
    let (dir, _store) = fixture_db(
        &[
            ("Function", "f.py::a", "a", "f.py", false),
            ("Function", "f.py::b", "b", "f.py", false),
            ("Test", "t.py::test_a", "test_a", "t.py", true),
        ],
        &[
            ("CALLS", "f.py::a", "f.py::b"),
            ("TESTED_BY", "f.py::a", "t.py::test_a"),
        ],
    );
    let g = AnalysisGraph::load(&open_conn(dir.path())).unwrap();
    assert_eq!(g.nodes.len(), 3);
    assert!(g.nodes.windows(2).all(|w| w[0].id < w[1].id));
    assert_eq!(g.edges[0].kind, "CALLS");
    assert_eq!(g.edges[1].kind, "TESTED_BY");
    assert_eq!(g.index_of("f.py::a"), Some(0));
    assert_eq!(g.by_id(g.nodes[2].id), Some(2));
    assert!(g.nodes[2].is_test);
}

#[test]
fn decorators_come_from_extra_as_list_or_string() {
    let (dir, _store) = fixture_db(
        &[
            ("Function", "f.py::a", "a", "f.py", false),
            ("Function", "f.py::b", "b", "f.py", false),
            ("Function", "f.py::c", "c", "f.py", false),
            ("Function", "f.py::d", "d", "f.py", false),
        ],
        &[],
    );
    let conn = open_conn(dir.path());
    for (qn, extra) in [
        ("f.py::a", r#"{"decorators":["app.get"]}"#),
        ("f.py::b", r#"{"decorators":"click.command"}"#),
        ("f.py::c", "{}"),
        ("f.py::d", "not json"),
    ] {
        conn.execute(
            "UPDATE nodes SET extra = ?1 WHERE qualified_name = ?2",
            [extra, qn],
        )
        .unwrap();
    }
    let g = AnalysisGraph::load(&conn).unwrap();
    assert_eq!(g.nodes[0].decorators, vec!["app.get"]);
    assert_eq!(g.nodes[1].decorators, vec!["click.command"]);
    assert!(g.nodes[2].decorators.is_empty());
    assert!(g.nodes[3].decorators.is_empty());
}

#[test]
fn round_half_even_matches_python() {
    // Verified against CPython: exact binary ties go to even, otherwise nearest.
    assert_eq!(round_half_even(0.125, 2), 0.12);
    assert_eq!(round_half_even(0.375, 2), 0.38);
    assert_eq!(round_half_even(0.12345, 4), 0.1235);
    assert_eq!(round_half_even(0.12355, 4), 0.1235);
}

#[test]
fn sanitize_name_strips_control_chars_and_caps_length() {
    assert_eq!(sanitize_name("a\u{1}b"), "ab");
    assert_eq!(sanitize_name(&"x".repeat(300)).chars().count(), 256);
}
