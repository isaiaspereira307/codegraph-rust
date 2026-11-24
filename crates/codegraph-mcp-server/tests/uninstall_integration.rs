// ABOUTME: Exercises `codegraph uninstall` against projects set up by `codegraph init`.
// ABOUTME: Checks that user content survives, runs are idempotent and failures write nothing.

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
        .env(
            "CODEGRAPH_CONFIG_PATH",
            directory.join("missing-config.toml"),
        )
        .env("CODEGRAPH_DEBUG", "0")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn ok(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

const AGENTS: &str = "# Team rules\nKeep team rules.\n";
const CLAUDE: &str = "# Claude rules\nKeep Claude rules.\n";
const SETTINGS: &str = r#"{"permissions":{"deny":["Bash(rm:*)"]}}"#;

fn project() -> tempfile::TempDir {
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join("AGENTS.md"), AGENTS).unwrap();
    fs::write(project.path().join("CLAUDE.md"), CLAUDE).unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();
    fs::write(project.path().join(".claude/settings.json"), SETTINGS).unwrap();
    project
}

fn read(project: &Path, name: &str) -> String {
    fs::read_to_string(project.join(name)).unwrap()
}

#[test]
fn uninstall_restores_the_project_and_keeps_user_content() {
    let project = project();
    ok(&run(
        &["init", "--hooks", "both", "--no-index"],
        project.path(),
    ));
    assert!(project.path().join(".codex/hooks.json").exists());

    ok(&run(&["uninstall"], project.path()));

    assert_eq!(read(project.path(), "AGENTS.md"), AGENTS);
    assert_eq!(read(project.path(), "CLAUDE.md"), CLAUDE);
    let settings: Value =
        serde_json::from_str(&read(project.path(), ".claude/settings.json")).unwrap();
    assert_eq!(settings, serde_json::from_str::<Value>(SETTINGS).unwrap());
    assert!(!project.path().join(".codex").exists());
}

#[test]
fn files_created_by_init_disappear_when_nothing_else_is_in_them() {
    let project = tempfile::tempdir().unwrap();
    ok(&run(
        &["init", "--hooks", "both", "--no-index"],
        project.path(),
    ));
    ok(&run(&["uninstall"], project.path()));
    let left: Vec<_> = fs::read_dir(project.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "left behind: {left:?}");
}

#[test]
fn hooks_flag_limits_removal_to_one_harness() {
    let project = project();
    ok(&run(
        &["init", "--hooks", "both", "--no-index"],
        project.path(),
    ));
    ok(&run(&["uninstall", "--hooks", "claude"], project.path()));
    assert_eq!(read(project.path(), "CLAUDE.md"), CLAUDE);
    assert!(read(project.path(), "AGENTS.md").contains("codegraph:begin"));
    assert!(project.path().join(".codex/hooks.json").exists());
    let settings = read(project.path(), ".claude/settings.json");
    assert!(!settings.contains("codegraph hooks emit"));
}

#[test]
fn second_run_dry_run_and_untouched_projects_change_nothing() {
    let untouched = project();
    let out = ok(&run(&["uninstall"], untouched.path()));
    assert!(out.contains("Nothing to remove"));
    assert_eq!(read(untouched.path(), "AGENTS.md"), AGENTS);

    let project = project();
    ok(&run(
        &["init", "--hooks", "both", "--no-index"],
        project.path(),
    ));
    let dry = ok(&run(&["uninstall", "--dry-run"], project.path()));
    assert!(dry.contains("Would update") && dry.contains("AGENTS.md"));
    assert!(read(project.path(), "AGENTS.md").contains("codegraph:begin"));

    ok(&run(&["uninstall"], project.path()));
    let again = ok(&run(&["uninstall"], project.path()));
    assert!(again.contains("Nothing to remove"));
}

#[test]
fn invalid_json_fails_before_any_file_changes() {
    let project = project();
    ok(&run(
        &["init", "--hooks", "both", "--no-index"],
        project.path(),
    ));
    fs::write(project.path().join(".codex/hooks.json"), "{ not json").unwrap();
    let before = read(project.path(), ".claude/settings.json");

    let output = run(&["uninstall"], project.path());
    assert!(!output.status.success());
    assert_eq!(read(project.path(), ".claude/settings.json"), before);
    assert!(read(project.path(), "AGENTS.md").contains("codegraph:begin"));
}

#[test]
fn purge_index_is_the_only_way_to_delete_the_graph() {
    let project = project();
    let index = project.path().join(".code-review-graph");
    fs::create_dir(&index).unwrap();
    fs::write(index.join("graph.db"), b"x").unwrap();
    ok(&run(&["uninstall"], project.path()));
    assert!(index.join("graph.db").exists());
    ok(&run(&["uninstall", "--purge-index"], project.path()));
    assert!(!index.exists());
}

#[test]
fn git_post_commit_block_is_removed_and_user_lines_stay() {
    let project = project();
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(project.path())
                .output()
                .unwrap()
                .status
                .success()
        )
    };
    git(&["init", "-q"]);
    let hook = project.path().join(".git/hooks/post-commit");
    fs::create_dir_all(hook.parent().unwrap()).unwrap();
    fs::write(&hook, "#!/bin/sh\necho mine\n").unwrap();
    ok(&run(&["hooks", "git", "install"], project.path()));
    assert!(fs::read_to_string(&hook).unwrap().contains("codegraph"));

    ok(&run(&["uninstall"], project.path()));
    assert_eq!(fs::read_to_string(&hook).unwrap(), "#!/bin/sh\necho mine\n");
}
