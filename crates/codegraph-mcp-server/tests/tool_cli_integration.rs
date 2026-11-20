// ABOUTME: Exercises the shipped CLI's public commands and project hook setup.
// ABOUTME: Checks output/exit contracts in isolated projects without real providers.

use serde_json::{Value, json};
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run(args: &[&str], input: Option<&str>, project: &std::path::Path) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_codegraph"))
        .args(args)
        .current_dir(project)
        .env("CODEGRAPH_CONFIG_PATH", project.join("missing-config.toml"))
        .env("CODEGRAPH_DEBUG", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    } else {
        drop(child.stdin.take());
    }
    child.wait_with_output().unwrap()
}

#[test]
fn guidance_and_tool_listing_do_not_load_configuration() {
    let project = tempfile::tempdir().unwrap();
    let output = run(&["tool", "instructions"], None, project.path());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let instructions = String::from_utf8(output.stdout).unwrap();
    assert!(instructions.contains("CodeGraph is a CLI tool available through Bash"));
    assert!(instructions.contains("using your Bash or shell execution tool"));
    assert!(instructions.contains("`command -v codegraph`"));
    let output = run(&["tool", "list"], None, project.path());
    assert!(output.status.success());
    let schemas: Value = serde_json::from_slice(&output.stdout).unwrap();
    let names: Vec<&str> = schemas
        .as_array()
        .unwrap()
        .iter()
        .map(|schema| schema["name"].as_str().unwrap())
        .collect();
    for tool in names.iter() {
        assert!(instructions.contains(tool), "guidance omits {tool}");
    }
    assert!(names.contains(&"find_nodes_by_name"));
    for args in [
        &["tool", "run"][..],
        &["tool", "run", "get_hub_nodes", "--timeout-secs", "0"][..],
    ] {
        let output = run(args, None, project.path());
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn hook_binary_emits_only_context_json_and_handles_invalid_events() {
    let project = tempfile::tempdir().unwrap();
    let input =
        json!({"hook_event_name": "SessionStart", "cwd": project.path(), "source": "compact"})
            .to_string();
    let output = run(&["hooks", "emit"], Some(&input), project.path());
    assert!(output.status.success());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        response["hookSpecificOutput"]["hookEventName"],
        "SessionStart"
    );
    assert!(
        response["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("codegraph tool run find_nodes_by_name")
    );
    let output = run(&["hooks", "emit"], Some("invalid JSON"), project.path());
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({})
    );
}

#[test]
fn hook_install_is_opt_in_and_preserves_project_settings() {
    let project = tempfile::tempdir().unwrap();
    let output = run(&["hooks", "install", "--dry-run"], None, project.path());
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(!project.path().join(".claude").exists());
    assert!(!project.path().join(".codex").exists());
    std::fs::create_dir(project.path().join(".claude")).unwrap();
    std::fs::write(
        project.path().join(".claude/settings.json"),
        r#"{"permissions":{"deny":["Bash(rm:*)"]}}"#,
    )
    .unwrap();
    let output = run(
        &["hooks", "install", "--harness", "both"],
        None,
        project.path(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let settings: Value = serde_json::from_slice(
        &std::fs::read(project.path().join(".claude/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(settings["permissions"]["deny"], json!(["Bash(rm:*)"]));
    let before = std::fs::read(project.path().join(".codex/hooks.json")).unwrap();
    assert!(
        run(&["hooks", "install"], None, project.path())
            .status
            .success()
    );
    assert_eq!(
        std::fs::read(project.path().join(".codex/hooks.json")).unwrap(),
        before
    );
}

#[test]
fn runtime_failure_has_json_stdout_and_nonzero_exit_status() {
    let project = tempfile::tempdir().unwrap();
    for (tool, params) in [
        ("get_hub_nodes", "not json"),
        ("get_hub_nodes", "[]"),
        ("no_such_tool", "{}"),
    ] {
        let output = run(
            &[
                "tool",
                "run",
                tool,
                "--params",
                params,
                "--timeout-secs",
                "30",
            ],
            None,
            project.path(),
        );
        assert_eq!(output.status.code(), Some(1));
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["error"]["tool"], tool);
        assert!(response["error"]["message"].as_str().is_some());
        assert!(!output.stderr.is_empty());
    }
}

/// Seeds `.code-review-graph/graph.db` with main → helper.
fn seed_graph(project: &std::path::Path) -> (String, String) {
    use codegraph_core::{CodeNode, EdgeType, Language, Location, NodeType};
    use codegraph_graph::{CodeEdge, sqlite_store::SqliteGraphStore};
    let node = |name: &str, line: u32| {
        CodeNode::new(
            name,
            Some(NodeType::Function),
            Some(Language::Python),
            Location {
                file_path: "app.py".into(),
                line,
                column: 0,
                end_line: Some(line + 1),
                end_column: None,
            },
        )
    };
    let (main, helper) = (node("main", 1), node("helper", 5));
    let ids = (main.id.to_string(), helper.id.to_string());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let store = SqliteGraphStore::open(project).unwrap();
        store
            .upsert_nodes_batch(&[main.clone(), helper.clone()])
            .await
            .unwrap();
        store
            .upsert_edges_batch(&[CodeEdge::new(main.id, helper.id, EdgeType::Calls)])
            .await
            .unwrap();
        store.finalize("full").await.unwrap();
    });
    ids
}

#[test]
fn tool_run_reads_the_sqlite_graph() {
    let project = tempfile::tempdir().unwrap();
    let (main_id, helper_id) = seed_graph(project.path());

    let output = run(
        &["tool", "run", "find_nodes_by_name", "--params", r#"{"name":"helper"}"#],
        None,
        project.path(),
    );
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    let found = &response["result"][0];
    assert_eq!(found["name"], "helper");
    assert_eq!(found["id"], helper_id);

    let params = json!({ "node_id": helper_id, "edge_type": "calls" }).to_string();
    let output = run(
        &["tool", "run", "get_reverse_dependencies", "--params", &params],
        None,
        project.path(),
    );
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["result"][0]["id"], main_id);
    assert_eq!(response["result"][0]["dependent_depth"], 1);
}

#[test]
fn db_check_reports_the_sqlite_graph() {
    let project = tempfile::tempdir().unwrap();
    seed_graph(project.path());
    let output = run(&["db-check"], None, project.path());
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(".code-review-graph/graph.db"), "{stdout}");
    assert!(stdout.contains("schema 15"), "{stdout}");
    assert!(stdout.contains("2 nodes, 1 edges, 1 files"), "{stdout}");
}
