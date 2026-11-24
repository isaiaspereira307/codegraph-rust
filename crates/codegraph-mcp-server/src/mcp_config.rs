// ABOUTME: Optional MCP registration for `codegraph init --mcp`: .mcp.json (Claude) and .codex/config.toml (Codex).
// ABOUTME: Never overwrites a user's own `codegraph` entry; the Codex block is text between markers so comments survive.

use crate::agent_hooks;
use crate::project_init::HookSelection;
use crate::uninstall::FileEdit;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const CODEX_BEGIN: &str = "# >>> codegraph mcp >>>";
const CODEX_END: &str = "# <<< codegraph mcp <<<";
const CODEX_BODY: &str =
    "[mcp_servers.codegraph]\ncommand = \"codegraph\"\nargs = [\"start\", \"stdio\"]\n";

fn entry() -> Value {
    json!({"command": "codegraph", "args": ["start", "stdio"]})
}

/// Adds `mcpServers.codegraph`, keeping every other key. A different entry already named
/// `codegraph` is the user's and is left alone.
pub(crate) fn merge_claude_mcp(original: &str) -> Result<String> {
    let mut root = parse_object(original)?;
    let servers = root
        .as_object_mut()
        .unwrap()
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("mcpServers must be a JSON object")?;
    servers.entry("codegraph").or_insert_with(entry);
    render(&root)
}

/// Removes `mcpServers.codegraph` only when it equals the entry `init` writes.
pub(crate) fn strip_claude_mcp(original: &str) -> Result<String> {
    let mut root = parse_object(original)?;
    let object = root.as_object_mut().unwrap();
    let Some(servers) = object.get_mut("mcpServers") else {
        return Ok(original.to_string());
    };
    let servers = servers
        .as_object_mut()
        .context("mcpServers must be a JSON object")?;
    if servers.get("codegraph") != Some(&entry()) {
        return Ok(original.to_string());
    }
    servers.remove("codegraph");
    if servers.is_empty() {
        object.remove("mcpServers");
    }
    render(&root)
}

fn parse_object(original: &str) -> Result<Value> {
    if original.trim().is_empty() {
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_str(original).context("Invalid JSON")?;
    value.as_object().context("Must be a JSON object")?;
    Ok(value)
}

fn render(value: &Value) -> Result<String> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    Ok(text)
}

fn codex_block() -> String {
    format!("{CODEX_BEGIN}\n{CODEX_BODY}{CODEX_END}\n")
}

fn has_codegraph_table(original: &str) -> Result<bool> {
    let table: toml::Table = toml::from_str(original).context("Invalid TOML")?;
    Ok(table
        .get("mcp_servers")
        .and_then(toml::Value::as_table)
        .is_some_and(|servers| servers.contains_key("codegraph")))
}

/// Appends the marked block unless `[mcp_servers.codegraph]` already exists (ours or the user's).
pub(crate) fn merge_codex_mcp(original: &str) -> Result<String> {
    if has_codegraph_table(original)? {
        return Ok(original.to_string());
    }
    let mut text = original.to_string();
    if !text.is_empty() {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push('\n');
    }
    text.push_str(&codex_block());
    Ok(text)
}

/// Removes the marked block (and the blank line before it); nothing else is touched.
pub(crate) fn strip_codex_mcp(original: &str) -> Result<String> {
    let mut head = String::new();
    let mut tail = String::new();
    let mut state = 0; // 0 before, 1 inside, 2 after
    for line in original.split_inclusive('\n') {
        let trimmed = line.trim_end();
        match state {
            0 if trimmed == CODEX_BEGIN => state = 1,
            0 => head.push_str(line),
            1 if trimmed == CODEX_END => state = 2,
            1 => {}
            _ => tail.push_str(line),
        }
    }
    match state {
        0 => Ok(original.to_string()),
        1 => bail!("CodeGraph MCP block in config.toml has no end marker; repair it first"),
        _ if tail.trim().is_empty() => {
            let head = head.trim_end();
            Ok(if head.is_empty() {
                String::new()
            } else {
                format!("{head}\n")
            })
        }
        _ => Ok(format!("{head}{tail}")),
    }
}

fn read(path: &Path) -> Result<Option<String>> {
    agent_hooks::reject_symlink(path)?;
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("Cannot read {}", path.display())),
    }
}

type Rewrite = fn(&str) -> Result<String>;

fn targets(project: &Path, selection: HookSelection, install: bool) -> Vec<(PathBuf, Rewrite)> {
    let mut out: Vec<(PathBuf, Rewrite)> = Vec::new();
    if selection.includes_claude() {
        let f: Rewrite = if install {
            merge_claude_mcp
        } else {
            strip_claude_mcp
        };
        out.push((project.join(".mcp.json"), f));
    }
    if selection.includes_codex() {
        let f: Rewrite = if install {
            merge_codex_mcp
        } else {
            strip_codex_mcp
        };
        out.push((project.join(".codex").join("config.toml"), f));
    }
    out
}

fn plan(
    project: &Path,
    selection: HookSelection,
    install: bool,
) -> Result<Vec<(PathBuf, FileEdit)>> {
    agent_hooks::reject_symlink(&project.join(".codex"))?;
    let mut plan = Vec::new();
    for (path, rewrite) in targets(project, selection, install) {
        let original = match read(&path)? {
            Some(text) => text,
            None if install => String::new(),
            None => continue,
        };
        let updated =
            rewrite(&original).with_context(|| format!("Cannot update {}", path.display()))?;
        if updated == original {
            continue;
        }
        let empty = updated.trim().is_empty()
            || (path.extension().is_some_and(|e| e == "json")
                && serde_json::from_str::<Value>(&updated)
                    .is_ok_and(|v| v.as_object().is_some_and(|o| o.is_empty())));
        plan.push((
            path,
            if empty && !install {
                FileEdit::Delete
            } else {
                FileEdit::Write(updated.into_bytes())
            },
        ));
    }
    Ok(plan)
}

pub(crate) fn plan_install(
    project: &Path,
    selection: HookSelection,
) -> Result<Vec<(PathBuf, FileEdit)>> {
    plan(project, selection, true)
}

pub(crate) fn plan_uninstall(
    project: &Path,
    selection: HookSelection,
) -> Result<Vec<(PathBuf, FileEdit)>> {
    plan(project, selection, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_merge_keeps_other_servers_and_strip_restores() {
        for original in [
            "",
            "{}",
            r#"{"mcpServers":{"other":{"command":"x"}},"extra":1}"#,
        ] {
            let merged = merge_claude_mcp(original).unwrap();
            let value: Value = serde_json::from_str(&merged).unwrap();
            assert_eq!(value["mcpServers"]["codegraph"], entry());
            let back = strip_claude_mcp(&merged).unwrap();
            let expected: Value =
                serde_json::from_str(if original.is_empty() { "{}" } else { original }).unwrap();
            assert_eq!(serde_json::from_str::<Value>(&back).unwrap(), expected);
        }
    }

    #[test]
    fn claude_users_own_codegraph_entry_is_kept() {
        let own = r#"{"mcpServers":{"codegraph":{"command":"/opt/cg"}}}"#;
        assert_eq!(
            merge_claude_mcp(own).unwrap(),
            render(&serde_json::from_str(own).unwrap()).unwrap()
        );
        assert_eq!(strip_claude_mcp(own).unwrap(), own);
        assert!(merge_claude_mcp("[1]").is_err());
    }

    #[test]
    fn codex_block_round_trips_and_keeps_comments() {
        for original in [
            "",
            "# my notes\nmodel = \"x\"\n",
            "model = \"x\"\n\n[mcp_servers.other]\ncommand = \"y\"\n",
        ] {
            let merged = merge_codex_mcp(original).unwrap();
            assert!(merged.contains("[mcp_servers.codegraph]"));
            assert_eq!(merge_codex_mcp(&merged).unwrap(), merged);
            assert_eq!(strip_codex_mcp(&merged).unwrap(), original);
        }
        let own = "[mcp_servers.codegraph]\ncommand = \"/opt/cg\"\n";
        assert_eq!(merge_codex_mcp(own).unwrap(), own);
        assert_eq!(strip_codex_mcp(own).unwrap(), own);
        assert!(merge_codex_mcp("not = [valid").is_err());
    }
}
