// ABOUTME: `codegraph doctor`: read-only checks of the binary, the graph database, hooks, skills and MCP entries.
// ABOUTME: Never writes, never uses the network; each check reports ok/warn/fail plus a one-line fix.

use crate::agent_hooks::{self, Harness};
use crate::skills::SKILLS;
use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
    pub fix: Option<String>,
}

fn check(
    name: &'static str,
    status: Status,
    detail: impl Into<String>,
    fix: Option<&str>,
) -> Check {
    Check {
        name,
        status,
        detail: detail.into(),
        fix: fix.map(str::to_string),
    }
}

/// Runs every check, in a fixed order, for the project at `project`.
pub fn run_checks(project: &Path) -> Vec<Check> {
    let project = project
        .canonicalize()
        .unwrap_or_else(|_| project.to_path_buf());
    vec![
        check_binary(),
        check_crg(&project),
        check_graph_db(&project),
        check_hooks(&project),
        check_git_hook(&project),
        check_skills(&project),
        check_mcp(&project),
        check_write_lock(&project),
    ]
}

fn on_path(program: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

fn check_binary() -> Check {
    let version = env!("CARGO_PKG_VERSION");
    let Some(found) = on_path("codegraph") else {
        return check(
            "binary",
            Status::Warn,
            format!("codegraph {version} is not on PATH (hooks and the git hook call `codegraph`)"),
            Some(
                "cargo install --path crates/codegraph-mcp-server --bin codegraph --all-features --force",
            ),
        );
    };
    let running = std::env::current_exe().and_then(|p| p.canonicalize()).ok();
    if let Some(running) = running
        && found.canonicalize().ok().as_ref() != Some(&running)
    {
        return check(
            "binary",
            Status::Warn,
            format!(
                "running {} but PATH resolves {}",
                running.display(),
                found.display()
            ),
            Some("make sure the codegraph you want comes first on PATH"),
        );
    }
    check(
        "binary",
        Status::Ok,
        format!("codegraph {version} at {}", found.display()),
        None,
    )
}

fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let token = text
        .split_whitespace()
        .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()))?;
    let mut parts = token.split('.').map(|p| {
        p.trim_matches(|c: char| !c.is_ascii_digit())
            .parse::<u32>()
            .ok()
    });
    Some((
        parts.next()??,
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
    ))
}

fn check_crg(project: &Path) -> Check {
    let Ok(output) = Command::new("code-review-graph").arg("--version").output() else {
        return check(
            "crg",
            Status::Ok,
            "code-review-graph is not installed",
            None,
        );
    };
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let Some(version) = parse_version(&text) else {
        return check("crg", Status::Ok, text.clone(), None);
    };
    if version >= (2, 4, 0) {
        return check("crg", Status::Ok, format!("{text} writes schema 15"), None);
    }
    let global_hooks = dirs::home_dir()
        .map(|home| home.join(".claude").join("settings.json"))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .is_some_and(|text| text.contains("code-review-graph"));
    let marker = project
        .join(".code-review-graph")
        .join("crg-hooks-off")
        .exists();
    if global_hooks && !marker {
        return check(
            "crg",
            Status::Warn,
            format!(
                "code-review-graph {text} writes schema 9 and its global hooks run in this project"
            ),
            Some(
                "touch .code-review-graph/crg-hooks-off (and skip the hooks there), or upgrade code-review-graph to 2.4.0+",
            ),
        );
    }
    check(
        "crg",
        Status::Ok,
        format!("{text} (schema 9), its hooks do not run here"),
        None,
    )
}

fn check_graph_db(project: &Path) -> Check {
    let path = project.join(".code-review-graph").join("graph.db");
    if !path.exists() {
        return check(
            "graph_db",
            Status::Fail,
            format!("{} does not exist", path.display()),
            Some("codegraph build ."),
        );
    }
    let conn = match Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(conn) => conn,
        Err(error) => {
            return check(
                "graph_db",
                Status::Fail,
                format!("cannot open {}: {error}", path.display()),
                Some("delete the file and run `codegraph build .`"),
            );
        }
    };
    let meta = |key: &str| -> Option<String> {
        conn.query_row("SELECT value FROM metadata WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .ok()
    };
    let expected = codegraph_graph::sqlite_store::SCHEMA_VERSION.to_string();
    match meta("schema_version") {
        Some(v) if v == expected => {}
        other => {
            return check(
                "graph_db",
                Status::Fail,
                format!(
                    "schema_version is {} (expected {expected})",
                    other.unwrap_or_else(|| "missing".into())
                ),
                Some(
                    "delete .code-review-graph/graph.db and run `codegraph build .` (see docs/architecture/storage-sqlite.md)",
                ),
            );
        }
    }
    let count = |sql: &str| {
        conn.query_row(sql, [], |r| r.get::<_, i64>(0))
            .unwrap_or(-1)
    };
    let nodes = count("SELECT count(*) FROM nodes");
    let edges = count("SELECT count(*) FROM edges");
    let suffixed =
        count("SELECT count(*) FROM nodes WHERE kind = 'File' AND qualified_name LIKE '%@L%'");
    let state = meta("build_state").unwrap_or_else(|| "unknown".into());
    let updated = meta("last_updated").unwrap_or_else(|| "never".into());
    let detail =
        format!("{nodes} nodes, {edges} edges, build_state={state}, last_updated={updated}");
    if state != "complete" {
        return check("graph_db", Status::Warn, detail, Some("codegraph build ."));
    }
    if suffixed > 0 {
        return check(
            "graph_db",
            Status::Warn,
            format!("{detail}; {suffixed} File nodes carry an old @L suffix"),
            Some("codegraph build ."),
        );
    }
    check("graph_db", Status::Ok, detail, None)
}

fn check_hooks(project: &Path) -> Check {
    let mut configured = Vec::new();
    for (harness, name) in [(Harness::Claude, "claude"), (Harness::Codex, "codex")] {
        match agent_hooks::hooks_configured(project, harness) {
            Ok(true) => configured.push(name),
            Ok(false) => {}
            Err(error) => {
                return check(
                    "hooks",
                    Status::Fail,
                    format!("{name}: {error}"),
                    Some("fix the invalid settings file"),
                );
            }
        }
    }
    if configured.is_empty() {
        return check(
            "hooks",
            Status::Warn,
            "no CodeGraph hooks in this project",
            Some("codegraph init . --hooks claude|codex|both --no-index"),
        );
    }
    check(
        "hooks",
        Status::Ok,
        format!("configured for {}", configured.join(", ")),
        None,
    )
}

fn check_git_hook(project: &Path) -> Check {
    if agent_hooks::git_hook_installed(project) {
        check("git_hook", Status::Ok, "post-commit hook installed", None)
    } else {
        check(
            "git_hook",
            Status::Ok,
            "post-commit hook not installed (optional; `codegraph hooks git install`)",
            None,
        )
    }
}

fn check_skills(project: &Path) -> Check {
    let dir = project.join(".claude").join("skills");
    let present = SKILLS
        .iter()
        .filter(|(name, _)| dir.join(name).join(".codegraph-managed").exists())
        .count();
    let claude = agent_hooks::hooks_configured(project, Harness::Claude).unwrap_or(false);
    match (present, claude) {
        (n, _) if n == SKILLS.len() => check(
            "skills",
            Status::Ok,
            format!("{n} review skills installed"),
            None,
        ),
        (_, true) => check(
            "skills",
            Status::Warn,
            format!("{present}/{} review skills installed", SKILLS.len()),
            Some("codegraph init . --hooks claude --no-index"),
        ),
        _ => check(
            "skills",
            Status::Ok,
            "not installed (Claude Code only)",
            None,
        ),
    }
}

fn check_mcp(project: &Path) -> Check {
    let mut found = Vec::new();
    if let Ok(text) = std::fs::read_to_string(project.join(".mcp.json"))
        && serde_json::from_str::<Value>(&text)
            .is_ok_and(|v| v["mcpServers"].get("codegraph").is_some())
    {
        found.push(".mcp.json");
    }
    if std::fs::read_to_string(project.join(".codex").join("config.toml"))
        .is_ok_and(|t| t.contains("[mcp_servers.codegraph]"))
    {
        found.push(".codex/config.toml");
    }
    if found.is_empty() {
        return check("mcp", Status::Ok, "no MCP registration (optional)", None);
    }
    if on_path("codegraph").is_none() {
        return check(
            "mcp",
            Status::Warn,
            format!(
                "registered in {} but `codegraph` is not on PATH",
                found.join(", ")
            ),
            Some("put codegraph on PATH"),
        );
    }
    check(
        "mcp",
        Status::Ok,
        format!("registered in {}", found.join(", ")),
        None,
    )
}

fn check_write_lock(project: &Path) -> Check {
    let path = project.join(".code-review-graph").join("write.lock");
    let pid = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| t.trim().parse::<u32>().ok());
    match pid {
        Some(pid) if Path::new(&format!("/proc/{pid}")).exists() => check(
            "write_lock",
            Status::Ok,
            format!("a build is running (pid {pid})"),
            None,
        ),
        Some(pid) => check(
            "write_lock",
            Status::Ok,
            format!("stale pid {pid} in write.lock (the kernel already released the lock)"),
            None,
        ),
        None => check("write_lock", Status::Ok, "no build running", None),
    }
}

pub fn render_table(checks: &[Check]) -> String {
    let mut out = String::new();
    for c in checks {
        let tag = match c.status {
            Status::Ok => "ok  ",
            Status::Warn => "warn",
            Status::Fail => "FAIL",
        };
        out.push_str(&format!("[{tag}] {:<10} {}\n", c.name, c.detail));
        if let Some(fix) = &c.fix {
            out.push_str(&format!("       fix: {fix}\n"));
        }
    }
    out
}

pub fn render_json(checks: &[Check]) -> Result<String> {
    Ok(serde_json::to_string_pretty(
        &serde_json::json!({ "checks": checks }),
    )?)
}

pub fn has_failure(checks: &[Check]) -> bool {
    checks.iter().any(|c| c.status == Status::Fail)
}
