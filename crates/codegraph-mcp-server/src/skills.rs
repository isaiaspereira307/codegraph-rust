// ABOUTME: The review skills `codegraph init` copies into a project's .claude/skills/.
// ABOUTME: A marker file records ownership so user skills of the same name are never overwritten or removed.

use crate::agent_hooks;
use crate::uninstall::FileEdit;
use anyhow::Result;
use std::path::{Path, PathBuf};

const MARKER: &str = ".codegraph-managed";

/// Skill name and `SKILL.md` contents; the single source is `rules-for-claude-code/skills/`.
pub(crate) const SKILLS: [(&str, &str); 3] = [
    (
        "review-changes",
        include_str!("../../../rules-for-claude-code/skills/review-changes/SKILL.md"),
    ),
    (
        "review-delta",
        include_str!("../../../rules-for-claude-code/skills/review-delta/SKILL.md"),
    ),
    (
        "review-pr",
        include_str!("../../../rules-for-claude-code/skills/review-pr/SKILL.md"),
    ),
];

fn skill_dir(project: &Path, name: &str) -> PathBuf {
    project.join(".claude").join("skills").join(name)
}

fn check_no_symlinks(project: &Path, dir: &Path) -> Result<()> {
    agent_hooks::reject_symlink(&project.join(".claude"))?;
    agent_hooks::reject_symlink(&project.join(".claude").join("skills"))?;
    agent_hooks::reject_symlink(dir)
}

/// Writes for every skill we own or that does not exist yet. A directory without our marker
/// belongs to the user: it is skipped with a warning.
pub(crate) fn plan_install(project: &Path) -> Result<Vec<(PathBuf, FileEdit)>> {
    let mut plan = Vec::new();
    for (name, contents) in SKILLS {
        let dir = skill_dir(project, name);
        check_no_symlinks(project, &dir)?;
        let marker = dir.join(MARKER);
        if dir.exists() && !marker.exists() {
            eprintln!(
                "Skipping skill {name}: {} exists and was not installed by CodeGraph",
                dir.display()
            );
            continue;
        }
        let file = dir.join("SKILL.md");
        if std::fs::read_to_string(&file).ok().as_deref() != Some(contents) {
            plan.push((file, FileEdit::Write(contents.as_bytes().to_vec())));
        }
        if !marker.exists() {
            plan.push((
                marker,
                FileEdit::Write(b"installed by codegraph init\n".to_vec()),
            ));
        }
    }
    Ok(plan)
}

/// Deletes the directories that carry our marker.
pub(crate) fn plan_uninstall(project: &Path) -> Result<Vec<(PathBuf, FileEdit)>> {
    let mut plan = Vec::new();
    for (name, _) in SKILLS {
        let dir = skill_dir(project, name);
        check_no_symlinks(project, &dir)?;
        if dir.join(MARKER).exists() {
            plan.push((dir, FileEdit::Delete));
        }
    }
    Ok(plan)
}
