// ABOUTME: Removes what `codegraph init` installed in a project, recognising only CodeGraph's own entries.
// ABOUTME: Plans every edit first (validating all files), then applies them; --dry-run stops after planning.

use crate::agent_hooks;
use crate::project_init::{self, HookSelection};
use crate::{mcp_config, skills};
use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// What to do with one path. `Delete` on a directory removes it recursively.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileEdit {
    Write(Vec<u8>),
    Delete,
}

/// Plans and (unless `dry_run`) applies the removal; returns the planned edits.
/// `hooks = None` means both harnesses.
pub fn run(
    project: &Path,
    hooks: Option<HookSelection>,
    purge_index: bool,
    dry_run: bool,
) -> Result<Vec<(PathBuf, FileEdit)>> {
    let project = agent_hooks::project_directory(project)?;
    let selection = hooks.unwrap_or(HookSelection::Both);
    let mut plan = Vec::new();

    for (included, dir, file) in [
        (selection.includes_claude(), ".claude", "settings.json"),
        (selection.includes_codex(), ".codex", "hooks.json"),
    ] {
        if included {
            plan.extend(plan_settings(&project, dir, file)?);
        }
    }
    for (included, name) in [
        (selection.includes_claude(), "CLAUDE.md"),
        (selection.includes_codex(), "AGENTS.md"),
    ] {
        if included {
            plan.extend(plan_guidance(&project, name)?);
        }
    }
    if selection.includes_claude() {
        plan.extend(skills::plan_uninstall(&project)?);
    }
    plan.extend(mcp_config::plan_uninstall(&project, selection)?);
    plan.extend(agent_hooks::plan_git_hook_removal(&project)?);
    if purge_index {
        let index = project.join(".code-review-graph");
        if index.exists() {
            plan.push((index, FileEdit::Delete));
        }
    }

    if !dry_run {
        for (path, edit) in &plan {
            apply(path, edit)?;
        }
    }
    Ok(plan)
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    agent_hooks::reject_symlink(path)?;
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Cannot read {}", path.display())),
    }
}

fn plan_settings(project: &Path, dir: &str, file: &str) -> Result<Option<(PathBuf, FileEdit)>> {
    let directory = project.join(dir);
    agent_hooks::reject_symlink(&directory)?;
    let path = directory.join(file);
    let Some(text) = read_optional(&path)? else {
        return Ok(None);
    };
    let original: Value = serde_json::from_str(&text)
        .with_context(|| format!("Invalid JSON in {}", path.display()))?;
    let stripped = agent_hooks::strip_settings(original.clone())
        .with_context(|| format!("Cannot update {}", path.display()))?;
    if stripped == original {
        return Ok(None);
    }
    if stripped.as_object().is_some_and(|o| o.is_empty()) {
        return Ok(Some((path, FileEdit::Delete)));
    }
    let mut bytes = serde_json::to_vec_pretty(&stripped)?;
    bytes.push(b'\n');
    Ok(Some((path, FileEdit::Write(bytes))))
}

fn plan_guidance(project: &Path, name: &str) -> Result<Option<(PathBuf, FileEdit)>> {
    let path = project.join(name);
    let Some(original) = read_optional(&path)? else {
        return Ok(None);
    };
    let stripped = project_init::strip_guidance(&original)
        .with_context(|| format!("Cannot update {}", path.display()))?;
    if stripped == original {
        return Ok(None);
    }
    Ok(Some(if stripped.trim().is_empty() {
        (path, FileEdit::Delete)
    } else {
        (path, FileEdit::Write(stripped.into_bytes()))
    }))
}

pub(crate) fn apply(path: &Path, edit: &FileEdit) -> Result<()> {
    match edit {
        FileEdit::Write(bytes) => agent_hooks::write_project_file(path, bytes),
        FileEdit::Delete => {
            if path.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)
                    .with_context(|| format!("Cannot remove {}", path.display()))?;
            }
            // Drop the config directories only when we emptied them (remove_dir fails otherwise).
            for dir in path.ancestors().skip(1).take(2) {
                let owned = match dir.file_name().and_then(|n| n.to_str()) {
                    Some(".claude" | ".codex") => true,
                    Some("skills") => dir
                        .parent()
                        .and_then(|p| p.file_name())
                        .is_some_and(|n| n == ".claude"),
                    _ => false,
                };
                if !owned || fs::remove_dir(dir).is_err() {
                    break;
                }
            }
            Ok(())
        }
    }
}
