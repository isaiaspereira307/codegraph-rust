// ABOUTME: Changed line ranges per file from `git diff -U0 <base>` and 90-day commit churn per file.
// ABOUTME: Reads only git's output; a directory that is not a git work tree is an error, never a fallback.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

/// One file of the diff with the new-side line ranges (inclusive) that changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    /// Path relative to the repository top-level, as git prints it.
    pub path: String,
    pub lines: Vec<(i32, i32)>,
}

/// Parses `git diff -U0` output. Each hunk `+c,d` marks lines c..c+d-1 of the new file. A
/// pure deletion (`d` = 0) marks the line just before the removal, so the node that lost lines
/// still matches. Deleted files (`+++ /dev/null`) are skipped: they have no new-side lines.
pub fn parse_zero_context_diff(diff: &str) -> Vec<ChangedFile> {
    let mut files: Vec<ChangedFile> = Vec::new();
    let mut current: Option<usize> = None;
    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("+++ ") {
            current = if path == "/dev/null" {
                None
            } else {
                let path = path.strip_prefix("b/").unwrap_or(path);
                files.push(ChangedFile {
                    path: path.to_string(),
                    lines: Vec::new(),
                });
                Some(files.len() - 1)
            };
        } else if let (Some(ix), Some(hunk)) = (current, line.strip_prefix("@@ ")) {
            let new_side = hunk.split_whitespace().find(|t| t.starts_with('+'));
            let Some((start, count)) = new_side.and_then(|t| parse_range(&t[1..])) else {
                continue;
            };
            let range = if count == 0 {
                let line = start.max(1);
                (line, line)
            } else {
                (start, start + count - 1)
            };
            files[ix].lines.push(range);
        }
    }
    files
}

/// `start[,count]`; a missing count means 1.
fn parse_range(text: &str) -> Option<(i32, i32)> {
    match text.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((text.parse().ok()?, 1)),
    }
}

/// Files and changed lines of the work tree against `base` (staged and unstaged tracked
/// changes; untracked files are not listed). Errors when `root` is not inside a git work tree
/// or when git rejects `base`.
pub fn changed_files(root: &Path, base: &str) -> Result<Vec<ChangedFile>, String> {
    if base.starts_with('-') {
        return Err(format!("invalid base_ref '{base}'"));
    }
    let inside = git(root, &["rev-parse", "--is-inside-work-tree"])
        .map_err(|_| format!("not a git work tree: {}", root.display()))?;
    if inside.trim() != "true" {
        return Err(format!("not a git work tree: {}", root.display()));
    }
    let diff = git(
        root,
        &[
            "-c",
            "core.quotePath=false",
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-renames",
            "-U0",
            base,
            "--",
        ],
    )?;
    Ok(parse_zero_context_diff(&diff))
}

const CHURN_BUDGET: Duration = Duration::from_secs(5);
const CHURN_CACHE_MAX_ENTRIES: usize = 32;

/// Process-wide churn state: results keyed by (root, HEAD), and roots whose git timed out.
/// Only a timeout is remembered: no commits or a missing `git` fail fast and may recover.
#[derive(Default)]
struct ChurnState {
    entries: HashMap<(PathBuf, String), HashMap<String, u32>>,
    timed_out: HashSet<PathBuf>,
    #[cfg(test)]
    log_runs: HashMap<PathBuf, usize>,
}

static CHURN: LazyLock<Mutex<ChurnState>> = LazyLock::new(Mutex::default);

fn churn_state() -> std::sync::MutexGuard<'static, ChurnState> {
    CHURN.lock().unwrap_or_else(|e| e.into_inner())
}

/// Forgets every cached churn result and remembered timeout (tests, long-lived processes).
pub fn clear_churn_cache() {
    let mut state = churn_state();
    state.entries.clear();
    state.timed_out.clear();
}

#[cfg(test)]
fn forget_root(root: &Path) {
    let mut state = churn_state();
    state.entries.retain(|(r, _), _| r != root);
    state.timed_out.remove(root);
}

#[cfg(test)]
fn git_log_runs(root: &Path) -> usize {
    churn_state().log_runs.get(root).copied().unwrap_or(0)
}

enum GitFailure {
    Timeout,
    Failed,
}

/// Runs `git -C root <args>` within `budget`; the reason for a failure goes to the debug log.
async fn run_git(root: &Path, args: &[&str], budget: Duration) -> Result<Vec<u8>, GitFailure> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("-C")
        .arg(root)
        .args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    // Churn is optional (`churn_status: "unavailable"`); keep the reason in the log.
    match tokio::time::timeout(budget, cmd.output()).await {
        Err(_) => {
            tracing::debug!("churn unavailable: git timed out after {budget:?}");
            Err(GitFailure::Timeout)
        }
        Ok(Err(error)) => {
            tracing::debug!("churn unavailable: failed to run git: {error}");
            Err(GitFailure::Failed)
        }
        Ok(Ok(output)) if !output.status.success() => {
            tracing::debug!(
                "churn unavailable: git exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
            Err(GitFailure::Failed)
        }
        Ok(Ok(output)) => Ok(output.stdout),
    }
}

/// Commits per file over the last 90 days (at most 2000 commits, 5 s budget), from one
/// `git log` run in `root`, cached per (root, HEAD). Keys are the paths relative to `root` and
/// the same paths joined to `root`, so a node's stored path (relative or absolute) finds its
/// count. `None` when git fails, times out or the repository has no commits; a timeout is
/// remembered until [`clear_churn_cache`].
pub async fn file_churn(root: &Path) -> Option<HashMap<String, u32>> {
    file_churn_with(root, CHURN_BUDGET).await
}

async fn file_churn_with(root: &Path, budget: Duration) -> Option<HashMap<String, u32>> {
    if churn_state().timed_out.contains(root) {
        return None;
    }
    let remember_timeout = |failure: GitFailure| {
        if matches!(failure, GitFailure::Timeout) {
            churn_state().timed_out.insert(root.to_path_buf());
        }
    };
    let head = match run_git(root, &["rev-parse", "HEAD"], budget).await {
        Ok(out) => String::from_utf8_lossy(&out).trim().to_string(),
        Err(failure) => {
            remember_timeout(failure);
            return None;
        }
    };
    if head.is_empty() {
        return None;
    }
    let key = (root.to_path_buf(), head);
    if let Some(hit) = churn_state().entries.get(&key) {
        return Some(hit.clone());
    }
    #[cfg(test)]
    {
        *churn_state()
            .log_runs
            .entry(root.to_path_buf())
            .or_default() += 1;
    }
    let stdout = match run_git(
        root,
        &[
            "-c",
            "core.quotePath=false",
            "log",
            "--since=90.days.ago",
            "--max-count=2000",
            "--no-renames",
            "--relative",
            "--name-only",
            "--format=",
            "-z",
            "--",
        ],
        budget,
    )
    .await
    {
        Ok(out) => out,
        Err(failure) => {
            remember_timeout(failure);
            return None;
        }
    };
    let text = String::from_utf8_lossy(&stdout);
    let mut counts: HashMap<String, u32> = HashMap::new();
    for path in text.split('\0').map(|p| p.trim_matches('\n')) {
        if path.is_empty() {
            continue;
        }
        *counts.entry(path.to_string()).or_default() += 1;
        *counts
            .entry(root.join(path).to_string_lossy().into_owned())
            .or_default() += 1;
    }
    let mut state = churn_state();
    if state.entries.len() >= CHURN_CACHE_MAX_ENTRIES {
        state.entries.clear();
    }
    state.entries.insert(key, counts.clone());
    Some(counts)
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hunks_per_file_and_skips_deleted_files() {
        let diff = "\
diff --git a/src/a.py b/src/a.py
--- a/src/a.py
+++ b/src/a.py
@@ -3 +3 @@ def f():
-old
+new
@@ -10,0 +11,3 @@
+x
+y
+z
@@ -20,2 +19,0 @@
-gone
-gone2
diff --git a/gone.py b/gone.py
--- a/gone.py
+++ /dev/null
@@ -1,2 +0,0 @@
-a
-b
";
        let files = parse_zero_context_diff(diff);
        assert_eq!(
            files,
            vec![ChangedFile {
                path: "src/a.py".to_string(),
                lines: vec![(3, 3), (11, 13), (19, 19)],
            }]
        );
    }

    #[test]
    fn rejects_non_repository_and_option_like_base() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            changed_files(dir.path(), "HEAD")
                .unwrap_err()
                .contains("not a git work tree")
        );
        assert!(changed_files(dir.path(), "--output=/tmp/x").is_err());
    }

    #[test]
    fn reports_lines_changed_since_base() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let run = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?} failed");
        };
        run(&["init", "-q"]);
        std::fs::write(root.join("m.py"), "a\nb\nc\n").unwrap();
        run(&["add", "m.py"]);
        run(&[
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-q",
            "-m",
            "base",
        ]);
        std::fs::write(root.join("m.py"), "a\nB\nc\nd\n").unwrap();

        let files = changed_files(root, "HEAD").unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "m.py");
        assert_eq!(files[0].lines, vec![(2, 2), (4, 4)]);
    }

    #[tokio::test]
    async fn churn_counts_commits_per_file_or_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(file_churn(root).await, None);
        let run = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?} failed");
        };
        run(&["init", "-q"]);
        // A repository without commits has no history to read.
        assert_eq!(file_churn(root).await, None);
        for (i, files) in [&["a.py", "b.py"][..], &["a.py"][..]].iter().enumerate() {
            for f in *files {
                std::fs::write(root.join(f), format!("{i}\n")).unwrap();
                run(&["add", f]);
            }
            run(&[
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-q",
                "-m",
                "c",
            ]);
        }
        let churn = file_churn(root).await.unwrap();
        assert_eq!(churn.get("a.py"), Some(&2));
        assert_eq!(churn.get("b.py"), Some(&1));
        let abs = root.join("a.py").to_string_lossy().into_owned();
        assert_eq!(churn.get(&abs), Some(&2));
    }

    fn git_ok(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.email=t@t", "-c", "user.name=t"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git_ok(dir.path(), &["init", "-q"]);
        dir
    }

    fn commit_file(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), format!("{name}\n{}", rand_suffix())).unwrap();
        git_ok(dir, &["add", "."]);
        git_ok(dir, &["commit", "-q", "-m", name]);
    }

    fn rand_suffix() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    #[tokio::test]
    async fn churn_is_cached_per_head() {
        let dir = repo();
        commit_file(dir.path(), "a.py");
        let first = file_churn(dir.path()).await.unwrap();
        let second = file_churn(dir.path()).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(first["a.py"], 1);
        assert_eq!(git_log_runs(dir.path()), 1);

        commit_file(dir.path(), "b.py");
        let third = file_churn(dir.path()).await.unwrap();
        assert_eq!(git_log_runs(dir.path()), 2);
        assert_eq!(third["b.py"], 1);
    }

    #[tokio::test]
    async fn two_repositories_do_not_share_entries() {
        let (one, two) = (repo(), repo());
        commit_file(one.path(), "only_one.py");
        commit_file(two.path(), "only_two.py");
        let a = file_churn(one.path()).await.unwrap();
        let b = file_churn(two.path()).await.unwrap();
        assert!(a.contains_key("only_one.py") && !a.contains_key("only_two.py"));
        assert!(b.contains_key("only_two.py") && !b.contains_key("only_one.py"));
    }

    #[tokio::test]
    async fn timeout_is_remembered_and_cleared() {
        let dir = repo();
        commit_file(dir.path(), "a.py");
        assert!(
            file_churn_with(dir.path(), std::time::Duration::ZERO)
                .await
                .is_none()
        );
        // Remembered: a normal budget is not tried again and no git log runs.
        assert!(file_churn(dir.path()).await.is_none());
        assert_eq!(git_log_runs(dir.path()), 0);

        forget_root(dir.path());
        assert!(file_churn(dir.path()).await.is_some());
    }

    #[tokio::test]
    async fn repository_without_commits_recovers_after_first_commit() {
        let dir = repo();
        assert!(file_churn(dir.path()).await.is_none());
        commit_file(dir.path(), "a.py");
        assert_eq!(file_churn(dir.path()).await.unwrap()["a.py"], 1);
    }
}
