// ABOUTME: Changed line ranges per file from `git diff -U0 <base>`, used by detect_changes.
// ABOUTME: Reads only git's output; a directory that is not a git work tree is an error, never a fallback.

use std::path::Path;
use std::process::Command;

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
}
