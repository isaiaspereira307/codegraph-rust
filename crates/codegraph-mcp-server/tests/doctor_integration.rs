// ABOUTME: Exercises `codegraph doctor` on temporary projects: missing, healthy, old-schema and suffixed graph databases.
// ABOUTME: HOME and PATH are isolated so the machine's own code-review-graph and hooks cannot leak in.

use rusqlite::Connection;
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output, Stdio},
};

fn run(arguments: &[&str], directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_codegraph"))
        .args(arguments)
        .current_dir(directory)
        .env("HOME", directory.join("fake-home"))
        .env("PATH", "/usr/bin:/bin")
        .env("CODEGRAPH_DEBUG", "0")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn checks(directory: &Path) -> (Vec<Value>, i32) {
    let output = run(&["doctor", "--json"], directory);
    let json: Value = serde_json::from_slice(&output.stdout).expect("valid JSON on stdout");
    (
        json["checks"].as_array().unwrap().clone(),
        output.status.code().unwrap(),
    )
}

fn status(checks: &[Value], name: &str) -> String {
    checks
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no check {name}"))["status"]
        .as_str()
        .unwrap()
        .to_string()
}

fn build(directory: &Path) {
    fs::write(directory.join("a.py"), "def f():\n    return 1\n").unwrap();
    let output = run(&["build", "."], directory);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn json_has_all_eight_checks_in_order() {
    let project = tempfile::tempdir().unwrap();
    let (list, _) = checks(project.path());
    let names: Vec<_> = list.iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "binary",
            "crg",
            "graph_db",
            "hooks",
            "git_hook",
            "skills",
            "mcp",
            "write_lock"
        ]
    );
}

#[test]
fn missing_graph_fails_with_a_fix_and_exit_code_one() {
    let project = tempfile::tempdir().unwrap();
    let (list, code) = checks(project.path());
    assert_eq!(status(&list, "graph_db"), "fail");
    assert_eq!(code, 1);
    let table = String::from_utf8(run(&["doctor"], project.path()).stdout).unwrap();
    assert!(table.contains("FAIL") && table.contains("fix: codegraph build ."));
}

#[test]
fn built_graph_is_ok_and_doctor_writes_nothing() {
    let project = tempfile::tempdir().unwrap();
    build(project.path());
    let before = fs::read(project.path().join(".code-review-graph/graph.db")).unwrap();
    let (list, code) = checks(project.path());
    assert_eq!(status(&list, "graph_db"), "ok");
    assert_eq!(code, 0);
    assert_eq!(
        fs::read(project.path().join(".code-review-graph/graph.db")).unwrap(),
        before
    );
}

#[test]
fn old_schema_residual_suffix_and_corrupt_database_are_reported() {
    let project = tempfile::tempdir().unwrap();
    build(project.path());
    let db = project.path().join(".code-review-graph/graph.db");

    let conn = Connection::open(&db).unwrap();
    conn.execute(
        "UPDATE nodes SET qualified_name = qualified_name || '@L1' WHERE kind = 'File'",
        [],
    )
    .unwrap();
    drop(conn);
    let (list, code) = checks(project.path());
    assert_eq!(status(&list, "graph_db"), "warn");
    assert_eq!(code, 0);

    let conn = Connection::open(&db).unwrap();
    conn.execute(
        "UPDATE metadata SET value = '9' WHERE key = 'schema_version'",
        [],
    )
    .unwrap();
    drop(conn);
    let (list, code) = checks(project.path());
    assert_eq!(status(&list, "graph_db"), "fail");
    assert_eq!(code, 1);
    let detail = list.iter().find(|c| c["name"] == "graph_db").unwrap()["detail"].to_string();
    assert!(detail.contains('9'));

    fs::write(&db, b"not a database").unwrap();
    let (list, _) = checks(project.path());
    assert_eq!(status(&list, "graph_db"), "fail");
}

#[test]
fn hooks_skills_and_mcp_follow_init() {
    let project = tempfile::tempdir().unwrap();
    let (list, _) = checks(project.path());
    assert_eq!(status(&list, "hooks"), "warn");

    let output = run(
        &["init", "--hooks", "both", "--mcp", "--no-index"],
        project.path(),
    );
    assert!(output.status.success());
    let (list, _) = checks(project.path());
    assert_eq!(status(&list, "hooks"), "ok");
    assert_eq!(status(&list, "skills"), "ok");
    let mcp = list.iter().find(|c| c["name"] == "mcp").unwrap();
    assert!(mcp["detail"].as_str().unwrap().contains(".mcp.json"));

    fs::remove_dir_all(project.path().join(".claude/skills/review-pr")).unwrap();
    let (list, _) = checks(project.path());
    assert_eq!(status(&list, "skills"), "warn");

    fs::write(project.path().join(".codex/hooks.json"), "{ nope").unwrap();
    let (list, code) = checks(project.path());
    assert_eq!(status(&list, "hooks"), "fail");
    assert_eq!(code, 1);
}
