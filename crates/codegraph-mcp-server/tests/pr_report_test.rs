// ABOUTME: Tests the PR report renderer, risk levels, exit-code gate and CLI failure path.
// ABOUTME: The fixture is a real `codegraph detect-changes` output (see report for how it was made).

use codegraph_mcp_server::pr_report::{
    FailOn, REPORT_MARKER, RiskLevel, gate_exit_code, render_report, risk_level,
};
use serde_json::{Value, json};
use std::process::Command;

const MAX: usize = 60_000;

fn fixture() -> Value {
    let raw = include_str!("fixtures/detect_changes_sample.json");
    serde_json::from_str(raw).unwrap()
}

#[test]
fn risk_level_boundaries() {
    assert_eq!(risk_level(0.69), RiskLevel::Low);
    assert_eq!(risk_level(0.70), RiskLevel::High);
    assert_eq!(risk_level(0.84), RiskLevel::High);
    assert_eq!(risk_level(0.85), RiskLevel::Critical);
    assert!(RiskLevel::Low < RiskLevel::High && RiskLevel::High < RiskLevel::Critical);
}

#[test]
fn fixture_report_has_marker_score_and_first_priority() {
    let changes = fixture();
    let report = render_report(&changes, MAX);
    assert!(report.starts_with(REPORT_MARKER));
    let score = changes["result"]["risk_score"].as_f64().unwrap();
    assert!(report.contains(&format!("{score:.2}")), "{report}");
    let first = changes["result"]["review_priorities"][0]["name"]
        .as_str()
        .unwrap();
    assert!(report.contains(first), "{report}");
    assert!(report.contains("Critical"), "{report}");
    assert!(report.contains("lib.py"), "{report}");
}

#[test]
fn empty_changes_report_has_no_risk_level() {
    let empty = json!({"result": {"risk_score": 0.0, "changed_files": [],
        "changed_nodes": [], "review_priorities": []}});
    let report = render_report(&empty, MAX);
    assert!(report.starts_with(REPORT_MARKER));
    assert!(report.contains("no changes analyzed"));
    for level in ["Low", "High", "Critical"] {
        assert!(!report.contains(level), "{report}");
    }
}

#[test]
fn oversized_input_is_truncated_on_a_char_boundary() {
    let nodes: Vec<Value> = (0..20_000)
        .map(|i| {
            json!({"id": format!("id{i}"), "kind": "Function", "name": format!("função_{i}_ñ"),
                "location": {"file_path": format!("src/módulo_{i}.rs"), "start_line": 1, "end_line": 2},
                "risk_score": 0.1})
        })
        .collect();
    let big = json!({"result": {"risk_score": 0.9,
        "changed_files": [{"path": "a.rs", "lines": [[1, 2]]}],
        "changed_nodes": nodes, "review_priorities": []}});
    for max in [MAX, 5_000, 777] {
        let report = render_report(&big, max);
        assert!(report.len() <= max, "{} > {max}", report.len());
        assert!(report.starts_with(REPORT_MARKER));
        assert!(report.contains("truncated"));
    }
}

#[test]
fn gate_exit_codes() {
    assert_eq!(gate_exit_code(0.99, FailOn::None), 0);
    assert_eq!(gate_exit_code(0.69, FailOn::High), 0);
    assert_eq!(gate_exit_code(0.70, FailOn::High), 3);
    assert_eq!(gate_exit_code(0.84, FailOn::Critical), 0);
    assert_eq!(gate_exit_code(0.85, FailOn::Critical), 3);
    assert_eq!(gate_exit_code(0.90, FailOn::High), 3);
}

#[test]
fn cli_without_graph_exits_1_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success()
        );
    };
    git(&["init", "-q"]);
    std::fs::write(dir.path().join("a.py"), "def f():\n    return 1\n").unwrap();
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.email=a@b.c",
        "-c",
        "user.name=t",
        "commit",
        "-qm",
        "init",
    ]);
    let out_file = dir.path().join("report.md");
    let output = Command::new(env!("CARGO_BIN_EXE_codegraph"))
        .args(["pr-report", "--base", "HEAD", "--output"])
        .arg(&out_file)
        .current_dir(dir.path())
        .env("CODEGRAPH_CONFIG_PATH", dir.path().join("missing.toml"))
        .env("CODEGRAPH_DEBUG", "0")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(!out_file.exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(REPORT_MARKER));
}
