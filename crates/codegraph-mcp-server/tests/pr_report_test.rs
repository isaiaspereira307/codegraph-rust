// ABOUTME: Tests the PR report renderer, risk levels, exit-code gate and CLI failure path.
// ABOUTME: The fixture is a real `codegraph detect-changes` output (see report for how it was made).

use codegraph_mcp_server::pr_report::{
    FailOn, REPORT_MARKER, RiskLevel, gate_exit_code, render_report, risk_level, write_report,
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
    assert_eq!(gate_exit_code(0.849999, FailOn::Critical), 0);
    assert_eq!(gate_exit_code(0.849999, FailOn::High), 3);
    assert_eq!(gate_exit_code(-1.0, FailOn::High), 0);
    assert_eq!(gate_exit_code(-1.0, FailOn::Critical), 0);
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

fn node(name: &str, path: &str, risk: f64) -> Value {
    json!({"id": "x", "kind": "Function", "name": name,
        "location": {"file_path": path, "start_line": 1, "end_line": 2}, "risk_score": risk})
}

#[test]
fn hostile_names_and_paths_cannot_inject_markup_or_columns() {
    let evil = node(
        "a`|<!-- codegraph-report -->",
        "src/<script>\nalert(1)\r</script>.rs",
        0.9,
    );
    let evil2 = node("``x`", "src/`y``.rs", 0.8);
    let changes = json!({"result": {"base_ref": "<!-- codegraph-report -->`", "risk_score": 0.9,
        "changed_files": [{"path": "src/<script>\nalert(1)\r</script>.rs", "lines": [[1, 2]]},
            {"path": "src/`y``.rs", "lines": [[1, 2]]}],
        "changed_nodes": [evil.clone(), evil2.clone()],
        "review_priorities": [evil, evil2]}});
    let report = render_report(&changes, MAX);
    assert_eq!(report.matches(REPORT_MARKER).count(), 1, "{report}");
    assert!(report.starts_with(REPORT_MARKER));
    assert!(!report.contains("<script"), "{report}");
    assert!(!report.contains("</script"), "{report}");
    assert_eq!(report.matches("<!--").count(), 1, "{report}");
    let rows: Vec<&str> = report.lines().filter(|l| l.starts_with('|')).collect();
    assert_eq!(rows.len(), 2 + 2, "{report}");
    for row in rows {
        assert_eq!(row.matches('|').count(), 5, "{row}");
    }
}

#[test]
fn details_section_is_capped_at_12000_bytes() {
    let nodes: Vec<Value> = (0..2_000)
        .map(|i| node(&format!("fn_{i:04}"), "a.rs", 0.1))
        .collect();
    let changes = json!({"result": {"risk_score": 0.1,
        "changed_files": [{"path": "a.rs", "lines": [[1, 2]]}],
        "changed_nodes": nodes, "review_priorities": []}});
    let report = render_report(&changes, MAX);
    let start = report.find("<details><summary>Changed functions").unwrap();
    let end = start + report[start..].find("</details>").unwrap();
    let body = &report[start..end];
    assert!(body.len() <= 12_000 + 200, "{}", body.len());
    assert!(body.contains("more entries truncated"));
}

#[test]
fn truncation_closes_open_details() {
    let nodes: Vec<Value> = (0..400)
        .map(|i| node(&format!("fn_{i:04}"), "a.rs", 0.1))
        .collect();
    let changes = json!({"result": {"risk_score": 0.1,
        "changed_files": [{"path": "a.rs", "lines": [[1, 2]]}],
        "changed_nodes": nodes, "review_priorities": []}});
    let report = render_report(&changes, 3_000);
    assert!(report.len() <= 3_000);
    assert_eq!(
        report.matches("<details>").count(),
        report.matches("</details>").count(),
        "{report}"
    );
}

#[test]
fn display_path_matches_on_component_boundary() {
    let changes = json!({"result": {"risk_score": 0.1,
        "changed_files": [{"path": "lib.py", "lines": [[1, 2]]}],
        "changed_nodes": [node("f", "/w/mylib.py", 0.1), node("g", "/w/lib.py", 0.1)],
        "review_priorities": [node("f", "/w/mylib.py", 0.1)]}});
    let report = render_report(&changes, MAX);
    assert!(report.contains("/w/mylib.py"), "{report}");
    assert!(report.contains("`g` (`Function`) in `lib.py`"), "{report}");
    assert!(
        report.contains("`f` (`Function`) in `/w/mylib.py`"),
        "{report}"
    );
}

#[test]
fn write_report_creates_missing_parent_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out/dir/report.md");
    write_report(&path, "hi").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hi");
    let bare = std::path::Path::new("report-bare-test.md");
    assert!(bare.parent().is_some_and(|p| p.as_os_str().is_empty()));
}
