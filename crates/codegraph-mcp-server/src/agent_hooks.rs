// ABOUTME: Opt-in project-local Claude Code and Codex lifecycle guidance.
// ABOUTME: Emits context without configuration loading, indexing, or provider calls.

use anyhow::{Context, Result, bail};
use clap::{Subcommand, ValueEnum};
use serde::Deserialize;
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub const CLI_INSTRUCTIONS: &str = include_str!("prompts/cli_instructions.md");
const HOOK_COMMAND: &str =
    "if command -v codegraph >/dev/null 2>&1; then codegraph hooks emit || true; fi";
const MAX_HOOK_INPUT_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Subcommand)]
pub enum HookCommand {
    /// Merge guidance hooks into project settings; never modify user-level settings.
    Install {
        #[arg(long, value_enum, default_value_t = Harness::Both)]
        harness: Harness,
        #[arg(long, default_value = ".")]
        project: PathBuf,
        /// Print proposed settings as JSON without creating or changing files.
        #[arg(long)]
        dry_run: bool,
    },
    /// Read a harness event from stdin and return non-blocking context JSON.
    Emit,
    /// Add or remove a marked post-commit block that runs `codegraph update --skip-flows` in
    /// the background (code-review-graph's hook also skips flows: `postprocess="minimal"`).
    /// Opt-in: nothing is installed unless this command is run.
    Git {
        #[command(subcommand)]
        action: GitHookAction,
    },
}

#[derive(Debug, Subcommand)]
pub enum GitHookAction {
    /// Append the marked block to the repository's post-commit hook (idempotent).
    Install {
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
    /// Remove only the marked block; delete the hook file if nothing else remains.
    Uninstall {
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
}

const GIT_BLOCK_START: &str = "# >>> codegraph post-commit >>>";
const GIT_BLOCK_END: &str = "# <<< codegraph post-commit <<<";
const GIT_BLOCK_BODY: &str = "if [ ! -f .code-review-graph/crg-hooks-off ] && command -v codegraph >/dev/null 2>&1; then\n  root=$(git rev-parse --show-toplevel) && (codegraph update \"$root\" --skip-flows >/dev/null 2>&1 &)\nfi";

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Harness {
    Claude,
    Codex,
    Both,
}

#[derive(Deserialize)]
struct HookInput {
    hook_event_name: String,
    cwd: Option<PathBuf>,
}

pub fn run(command: &HookCommand) -> Result<()> {
    match command {
        HookCommand::Install {
            harness,
            project,
            dry_run,
        } => {
            let changes = install(project, *harness, *dry_run)?;
            if *dry_run {
                println!("{}", serde_json::to_string_pretty(&changes)?);
            } else {
                for change in changes {
                    println!(
                        "{} {}",
                        if change.changed {
                            "Updated"
                        } else {
                            "Already configured:"
                        },
                        change.path.display()
                    );
                }
            }
            Ok(())
        }
        HookCommand::Emit => {
            // Hooks fail open on invalid/oversized input and unsupported events.
            let mut input = String::new();
            let response = if io::stdin()
                .take(MAX_HOOK_INPUT_BYTES + 1)
                .read_to_string(&mut input)
                .is_ok()
                && input.len() as u64 <= MAX_HOOK_INPUT_BYTES
            {
                hook_response(&input)
            } else {
                json!({})
            };
            println!("{response}");
            Ok(())
        }
        HookCommand::Git { action } => match action {
            GitHookAction::Install { project } => {
                let (path, changed) = install_git_hook(project)?;
                println!(
                    "{} {}",
                    if changed {
                        "Installed:"
                    } else {
                        "Already configured:"
                    },
                    path.display()
                );
                Ok(())
            }
            GitHookAction::Uninstall { project } => {
                let (path, changed) = uninstall_git_hook(project)?;
                println!(
                    "{} {}",
                    if changed {
                        "Removed block from:"
                    } else {
                        "Not installed:"
                    },
                    path.display()
                );
                Ok(())
            }
        },
    }
}

fn git_block() -> String {
    format!("{GIT_BLOCK_START}\n{GIT_BLOCK_BODY}\n{GIT_BLOCK_END}\n")
}

/// Refuses a hook whose start marker has no end marker: `strip_git_block` would otherwise
/// drop every user line after it.
fn check_git_block(text: &str) -> Result<()> {
    let mut open = false;
    for line in text.lines() {
        match line.trim_end() {
            GIT_BLOCK_START if open => {
                bail!("CodeGraph git hook markers are duplicated; repair the block first")
            }
            GIT_BLOCK_START => open = true,
            GIT_BLOCK_END => open = false,
            _ => {}
        }
    }
    if open {
        bail!(
            "CodeGraph git hook block has no end marker ({GIT_BLOCK_END}); repair the post-commit hook first"
        );
    }
    Ok(())
}

/// Remove the marked block (every occurrence) and keep all other lines unchanged.
fn strip_git_block(text: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end();
        if skipping {
            if trimmed == GIT_BLOCK_END {
                skipping = false;
            }
        } else if trimmed == GIT_BLOCK_START {
            skipping = true;
        } else {
            out.push_str(line);
        }
    }
    out
}

fn with_git_block(existing: &str) -> String {
    let mut base = strip_git_block(existing);
    if base.trim().is_empty() {
        base = "#!/bin/sh\n".to_string();
    } else if !base.ends_with('\n') {
        base.push('\n');
    }
    base.push_str(&git_block());
    base
}

fn post_commit_path(project: &Path) -> Result<PathBuf> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(project)
        .args(["rev-parse", "--git-path", "hooks/post-commit"])
        .output()
        .context("Cannot run git")?;
    if !output.status.success() {
        bail!("{} is not a git repository", project.display());
    }
    let relative = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let path = PathBuf::from(relative);
    Ok(if path.is_absolute() {
        path
    } else {
        project.join(path)
    })
}

fn read_hook(path: &Path) -> Result<Option<String>> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!(
            "Refusing to write git hook through symlink {}",
            path.display()
        );
    }
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn install_git_hook(project: &Path) -> Result<(PathBuf, bool)> {
    let path = post_commit_path(project)?;
    let existing = read_hook(&path)?.unwrap_or_default();
    check_git_block(&existing)?;
    let updated = with_git_block(&existing);
    if updated == existing {
        return Ok((path, false));
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&path, updated)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    }
    Ok((path, true))
}

fn uninstall_git_hook(project: &Path) -> Result<(PathBuf, bool)> {
    let path = post_commit_path(project)?;
    let Some(existing) = read_hook(&path)? else {
        return Ok((path, false));
    };
    check_git_block(&existing)?;
    let stripped = strip_git_block(&existing);
    if stripped == existing {
        return Ok((path, false));
    }
    if stripped.trim().is_empty() || stripped.trim() == "#!/bin/sh" {
        fs::remove_file(&path)?;
    } else {
        fs::write(&path, stripped)?;
    }
    Ok((path, true))
}

/// Produce only guidance; never evaluate prompts, tool arguments, or transcript paths.
pub fn hook_response(input: &str) -> Value {
    let Ok(input) = serde_json::from_str::<HookInput>(input) else {
        return json!({});
    };
    if !matches!(
        input.hook_event_name.as_str(),
        "SessionStart" | "SubagentStart"
    ) {
        return json!({});
    }
    let mut instructions = CLI_INSTRUCTIONS.to_string();
    if let Some(cwd) = input.cwd.as_deref() {
        if let Some(root) = project_root(cwd) {
            // Quote for the POSIX shell used by the installed hook command.
            let quoted_root = format!("'{}'", root.to_string_lossy().replace('\'', "'\\''"));
            instructions.push_str(&format!(
                "\nSession project root: {}. When calling from a subdirectory, append `--project {quoted_root}` to each `codegraph tool run` command.\n",
                root.display()
            ));
        }
    }
    json!({"hookSpecificOutput": {
        "hookEventName": input.hook_event_name, "additionalContext": instructions
    }})
}

fn project_root(cwd: &Path) -> Option<PathBuf> {
    let cwd = cwd.canonicalize().ok()?;
    if !cwd.is_dir() {
        return None;
    }
    cwd.ancestors()
        .find(|path| path.join(".git").exists())
        .map(Path::to_path_buf)
        .or(Some(cwd))
}

#[derive(serde::Serialize)]
pub struct HookChange {
    pub(crate) path: PathBuf,
    pub(crate) changed: bool,
    settings: Value,
}

pub(crate) fn project_directory(project: &Path) -> Result<PathBuf> {
    let project = project
        .canonicalize()
        .context("Cannot resolve hook project directory")?;
    if !project.is_dir() {
        bail!("Hook project must be a directory");
    }
    if dirs::home_dir()
        .and_then(|home| home.canonicalize().ok())
        .as_ref()
        == Some(&project)
    {
        bail!("Hooks must be installed in a project directory, not the user home directory");
    }
    Ok(project)
}

/// Read and validate both configurations before writing either one.
pub fn install(project: &Path, harness: Harness, dry_run: bool) -> Result<Vec<HookChange>> {
    let project = project_directory(project)?;
    let paths: &[(&str, &str)] = match harness {
        Harness::Claude => &[(".claude", "settings.json")],
        Harness::Codex => &[(".codex", "hooks.json")],
        Harness::Both => &[(".claude", "settings.json"), (".codex", "hooks.json")],
    };
    let mut changes = Vec::new();
    for (directory, filename) in paths {
        let directory = project.join(directory);
        let path = directory.join(filename);
        reject_symlink(&directory)?;
        reject_symlink(&path)?;
        let original = match fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content)
                .with_context(|| format!("Invalid JSON in {}", path.display()))?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => json!({}),
            Err(error) => {
                return Err(error).with_context(|| format!("Cannot read {}", path.display()));
            }
        };
        let inline = if directory.ends_with(".codex") {
            let config = directory.join("config.toml");
            reject_symlink(&config)?;
            match fs::read_to_string(&config) {
                Ok(contents) => Some(serde_json::to_value(
                    toml::from_str::<toml::Value>(&contents)
                        .with_context(|| format!("Invalid TOML in {}", config.display()))?,
                )?),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(error).with_context(|| format!("Cannot read {}", config.display()));
                }
            }
        } else {
            None
        };
        let settings = merge_settings(original.clone(), inline.as_ref())
            .with_context(|| format!("Cannot merge {}", path.display()))?;
        changes.push(HookChange {
            path,
            changed: original != settings,
            settings,
        });
    }
    if !dry_run {
        apply_changes(&changes)?;
    }
    Ok(changes)
}

pub(crate) fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!(
                "Refusing to write project hooks through symlink {}",
                path.display()
            )
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn configured_event(settings: &Value, event: &str) -> bool {
    settings
        .get("hooks")
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .is_some_and(|groups| {
            groups.iter().any(|group| {
                group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|handlers| {
                        handlers.iter().any(|handler| {
                            handler.get("command").and_then(Value::as_str) == Some(HOOK_COMMAND)
                        })
                    })
            })
        })
}

fn merge_settings(mut settings: Value, inline: Option<&Value>) -> Result<Value> {
    // Validate the existing JSON even when both events are configured in inline TOML.
    settings
        .as_object()
        .context("Settings must be a JSON object")?;
    if let Some(hooks) = settings.get("hooks") {
        hooks.as_object().context("hooks must be a JSON object")?;
        for event in ["SessionStart", "SubagentStart"] {
            if let Some(groups) = hooks.get(event) {
                for group in groups
                    .as_array()
                    .with_context(|| format!("{event} must be an array"))?
                {
                    group
                        .get("hooks")
                        .and_then(Value::as_array)
                        .with_context(|| {
                            format!("{event} matcher group must have a hooks array")
                        })?;
                }
            }
        }
    }
    let installed = |event| {
        configured_event(&settings, event)
            || inline.is_some_and(|inline| configured_event(inline, event))
    };
    let missing: Vec<_> = ["SessionStart", "SubagentStart"]
        .into_iter()
        .filter(|event| !installed(event))
        .collect();
    if missing.is_empty() {
        return Ok(settings);
    }
    let object = settings
        .as_object_mut()
        .context("Settings must be a JSON object")?;
    let hooks = object
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("hooks must be a JSON object")?;
    for event in missing {
        let groups = hooks
            .entry(event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .with_context(|| format!("{event} must be an array"))?;
        let mut group = json!({"hooks": [{
            "type": "command", "command": HOOK_COMMAND, "timeout": 5
        }]});
        if event == "SessionStart" {
            group["matcher"] = json!("^(startup|resume|clear|compact)$");
        }
        groups.push(group);
    }
    Ok(settings)
}

/// Inverse of [`merge_settings`]: drops only CodeGraph's handlers and the groups, events and
/// `hooks` object that this leaves empty. Anything else in the file is kept as is.
pub(crate) fn strip_settings(mut settings: Value) -> Result<Value> {
    settings
        .as_object()
        .context("Settings must be a JSON object")?;
    let Some(hooks_value) = settings.get_mut("hooks") else {
        return Ok(settings);
    };
    let hooks = hooks_value
        .as_object_mut()
        .context("hooks must be a JSON object")?;
    let mut touched = false;
    for event in ["SessionStart", "SubagentStart"] {
        let Some(groups_value) = hooks.get_mut(event) else {
            continue;
        };
        let groups = groups_value
            .as_array_mut()
            .with_context(|| format!("{event} must be an array"))?;
        let mut event_touched = false;
        for group in groups.iter_mut() {
            let handlers = group
                .get_mut("hooks")
                .and_then(Value::as_array_mut)
                .with_context(|| format!("{event} matcher group must have a hooks array"))?;
            let before = handlers.len();
            handlers.retain(|h| h.get("command").and_then(Value::as_str) != Some(HOOK_COMMAND));
            event_touched |= handlers.len() != before;
        }
        if event_touched {
            touched = true;
            groups.retain(|g| {
                g.get("hooks")
                    .and_then(Value::as_array)
                    .is_some_and(|h| !h.is_empty())
            });
            if groups.is_empty() {
                hooks.remove(event);
            }
        }
    }
    if touched && hooks.is_empty() {
        settings.as_object_mut().unwrap().remove("hooks");
    }
    Ok(settings)
}

/// True when `harness` already has every CodeGraph hook (project JSON or inline Codex TOML).
pub(crate) fn hooks_configured(project: &Path, harness: Harness) -> Result<bool> {
    Ok(install(project, harness, true)?.iter().all(|c| !c.changed))
}

/// True when the git post-commit block is present.
pub(crate) fn git_hook_installed(project: &Path) -> bool {
    plan_git_hook_removal(project).ok().flatten().is_some()
}

/// The edit for removing the git post-commit block, or `None` when there is no repository or
/// no block. Mirrors [`uninstall_git_hook`] without writing.
pub(crate) fn plan_git_hook_removal(
    project: &Path,
) -> Result<Option<(PathBuf, crate::uninstall::FileEdit)>> {
    let Ok(path) = post_commit_path(project) else {
        return Ok(None);
    };
    let Some(existing) = read_hook(&path)? else {
        return Ok(None);
    };
    check_git_block(&existing)?;
    let stripped = strip_git_block(&existing);
    if stripped == existing {
        return Ok(None);
    }
    let edit = if stripped.trim().is_empty() || stripped.trim() == "#!/bin/sh" {
        crate::uninstall::FileEdit::Delete
    } else {
        crate::uninstall::FileEdit::Write(stripped.into_bytes())
    };
    Ok(Some((path, edit)))
}

pub(crate) fn apply_changes(changes: &[HookChange]) -> Result<()> {
    for change in changes {
        if change.changed {
            let mut contents = serde_json::to_vec_pretty(&change.settings)?;
            contents.push(b'\n');
            write_project_file(&change.path, &contents)?;
        }
    }
    Ok(())
}

pub(crate) fn write_project_file(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().context("Settings have no parent directory")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".codegraph-hooks-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        if let Ok(metadata) = fs::metadata(path) {
            file.set_permissions(metadata.permissions())?;
        }
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("Cannot write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_inline_hooks_are_reused_and_only_missing_events_are_installed() {
        let project = tempfile::tempdir().unwrap();
        let directory = project.path().join(".codex");
        fs::create_dir(&directory).unwrap();
        let mut inline = json!({"model": "custom-model", "hooks": {
            "SessionStart": [{"hooks": [{"type": "command", "command": HOOK_COMMAND}]}]
        }});
        let config = directory.join("config.toml");
        fs::write(&config, toml::to_string(&inline).unwrap()).unwrap();
        let changes = install(project.path(), Harness::Codex, false).unwrap();
        assert!(changes[0].settings["hooks"].get("SessionStart").is_none());
        assert!(changes[0].settings["hooks"].get("SubagentStart").is_some());
        assert!(!install(project.path(), Harness::Codex, true).unwrap()[0].changed);
        fs::remove_file(directory.join("hooks.json")).unwrap();
        inline["hooks"]["SubagentStart"] =
            json!([{"hooks": [{"type": "command", "command": HOOK_COMMAND}]}]);
        let contents = toml::to_string(&inline).unwrap();
        fs::write(&config, &contents).unwrap();
        assert!(!install(project.path(), Harness::Codex, false).unwrap()[0].changed);
        assert!(!directory.join("hooks.json").exists());
        assert_eq!(fs::read_to_string(config).unwrap(), contents);
    }

    #[test]
    fn guidance_is_restored_for_sessions_and_subagents_without_blocking() {
        for event in ["SessionStart", "SubagentStart"] {
            let output = hook_response(&json!({"hook_event_name": event}).to_string());
            assert_eq!(output["hookSpecificOutput"]["hookEventName"], event);
            let context = output["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap();
            assert!(context.contains("codegraph tool run find_nodes_by_name"));
            assert!(output.get("decision").is_none());
            assert!(output.get("continue").is_none());
        }
        for input in ["not json", "{}", r#"{"hook_event_name":"PreToolUse"}"#] {
            assert_eq!(hook_response(input), json!({}));
        }
    }

    #[test]
    fn merge_preserves_permissions_and_existing_hooks_and_is_idempotent() {
        let existing = json!({
            "permissions": {"allow": ["Bash(cargo test:*)"]},
            "env": {"CUSTOM_SETTING": "keep"},
            "hooks": {"SessionStart": [{"matcher": "startup", "hooks": [{"type": "command", "command": "echo existing"}]}]}
        });
        let merged = merge_settings(existing.clone(), None).unwrap();
        assert_eq!(merged["permissions"], existing["permissions"]);
        assert_eq!(merged["env"], existing["env"]);
        assert_eq!(
            merged["hooks"]["SessionStart"][0],
            existing["hooks"]["SessionStart"][0]
        );
        assert_eq!(
            merged["hooks"]["SessionStart"][1]["matcher"],
            "^(startup|resume|clear|compact)$"
        );
        assert_eq!(merge_settings(merged.clone(), None).unwrap(), merged);
    }

    #[test]
    fn dry_run_creates_nothing_and_invalid_second_config_changes_neither() {
        let project = tempfile::tempdir().unwrap();
        let changes = install(project.path(), Harness::Both, true).unwrap();
        assert_eq!(changes.len(), 2);
        assert!(!project.path().join(".claude").exists());
        assert!(!project.path().join(".codex").exists());
        fs::create_dir(project.path().join(".codex")).unwrap();
        fs::write(project.path().join(".codex/hooks.json"), "invalid json").unwrap();
        assert!(install(project.path(), Harness::Both, false).is_err());
        assert!(!project.path().join(".claude").exists());
        assert_eq!(
            fs::read_to_string(project.path().join(".codex/hooks.json")).unwrap(),
            "invalid json"
        );
    }

    #[test]
    fn install_is_repeatable_and_guidance_finds_root_from_subdirectory() {
        let project = tempfile::tempdir().unwrap();
        fs::create_dir(project.path().join(".git")).unwrap();
        fs::create_dir(project.path().join("nested")).unwrap();
        let first = install(project.path(), Harness::Both, false).unwrap();
        assert!(first.iter().all(|change| change.changed));
        let before = fs::read(project.path().join(".codex/hooks.json")).unwrap();
        let second = install(project.path(), Harness::Both, false).unwrap();
        assert!(second.iter().all(|change| !change.changed));
        assert_eq!(
            fs::read(project.path().join(".codex/hooks.json")).unwrap(),
            before
        );
        let output = hook_response(
            &json!({"hook_event_name": "SessionStart", "cwd": project.path().join("nested")})
                .to_string(),
        );
        let root = project.path().canonicalize().unwrap();
        assert!(
            output["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap()
                .contains(&format!("--project '{}'", root.display()))
        );
    }

    #[cfg(unix)]
    #[test]
    fn installed_shell_command_fails_open_for_missing_or_old_binaries() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::Command;

        let directory = tempfile::tempdir().unwrap();
        let invoke = || {
            Command::new("/bin/sh")
                .args(["-c", HOOK_COMMAND])
                .env("PATH", directory.path())
                .output()
                .unwrap()
        };
        let missing = invoke();
        assert!(missing.status.success());
        assert!(missing.stdout.is_empty());
        let binary = directory.path().join("codegraph");
        fs::write(&binary, "#!/bin/sh\nexit 2\n").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(invoke().status.success());
    }

    #[cfg(unix)]
    #[test]
    fn installer_rejects_symlinks_to_other_configuration() {
        let project = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), project.path().join(".claude")).unwrap();
        assert!(install(project.path(), Harness::Claude, false).is_err());
        assert!(!outside.path().join("settings.json").exists());
    }

    #[test]
    fn strip_settings_undoes_merge_and_keeps_user_content() {
        for original in [
            json!({}),
            json!({"permissions": {"allow": ["Bash(ls)"]}}),
            json!({"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}}),
        ] {
            let merged = merge_settings(original.clone(), None).unwrap();
            assert_ne!(merged, original);
            assert_eq!(strip_settings(merged).unwrap(), original);
        }
        assert_eq!(strip_settings(json!({})).unwrap(), json!({}));
        assert!(strip_settings(json!({"hooks": []})).is_err());
    }

    #[test]
    fn git_block_without_end_marker_is_refused() {
        let broken = format!("#!/bin/sh\necho mine\n{GIT_BLOCK_START}\necho after\n");
        assert!(check_git_block(&broken).is_err());
        assert!(check_git_block(&with_git_block("#!/bin/sh\necho mine\n")).is_ok());
        assert!(check_git_block("#!/bin/sh\necho mine\n").is_ok());
    }

    #[test]
    fn git_block_on_empty_hook_adds_shebang_and_is_idempotent() {
        let once = with_git_block("");
        assert!(once.starts_with("#!/bin/sh\n"));
        assert!(once.contains(GIT_BLOCK_START) && once.contains(GIT_BLOCK_END));
        assert_eq!(with_git_block(&once), once);
    }

    #[test]
    fn git_block_keeps_user_lines_and_strips_back_exactly() {
        let user = "#!/bin/sh\necho hi\n";
        let with = with_git_block(user);
        assert!(with.contains("echo hi"));
        assert_eq!(strip_git_block(&with), user);
    }

    #[test]
    fn git_block_appends_newline_to_user_hook_without_one() {
        let with = with_git_block("#!/bin/sh\necho hi");
        assert_eq!(strip_git_block(&with), "#!/bin/sh\necho hi\n");
    }
}
