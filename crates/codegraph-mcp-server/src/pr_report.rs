// ABOUTME: Renders a detect_changes result as a size-capped markdown PR comment.
// ABOUTME: Also holds the risk-level thresholds and the --fail-on-risk exit-code gate.

use crate::tool_cli::{self, ToolTarget};
use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// First line of every report; lets the CI action find and update its own comment.
pub const REPORT_MARKER: &str = "<!-- codegraph-report -->";
/// Cap for the whole report (GitHub comments allow 65,536 characters).
pub const MAX_REPORT_BYTES: usize = 60_000;
/// Cap for each collapsible `<details>` section.
pub const MAX_DETAILS_BYTES: usize = 12_000;
/// Exit code when the risk reached `--fail-on-risk` (clap already uses 2 for usage errors).
pub const EXIT_RISK_GATE: i32 = 3;
/// Exit code when the analysis itself failed; no report is written.
pub const EXIT_ANALYSIS_FAILED: i32 = 1;

const HIGH_THRESHOLD: f64 = 0.70;
const CRITICAL_THRESHOLD: f64 = 0.85;
const PRIORITY_ROWS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskLevel {
    Low,
    High,
    Critical,
}

impl RiskLevel {
    fn label(self) -> &'static str {
        match self {
            RiskLevel::Low => "Low",
            RiskLevel::High => "High",
            RiskLevel::Critical => "Critical",
        }
    }
}

pub fn risk_level(score: f64) -> RiskLevel {
    if score >= CRITICAL_THRESHOLD {
        RiskLevel::Critical
    } else if score >= HIGH_THRESHOLD {
        RiskLevel::High
    } else {
        RiskLevel::Low
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FailOn {
    None,
    High,
    Critical,
}

/// Exit code for a risk score under the given gate: 0 passes, 3 reached the threshold.
pub fn gate_exit_code(score: f64, fail_on: FailOn) -> i32 {
    let reached = match fail_on {
        FailOn::None => false,
        FailOn::High => risk_level(score) >= RiskLevel::High,
        FailOn::Critical => risk_level(score) >= RiskLevel::Critical,
    };
    if reached { EXIT_RISK_GATE } else { 0 }
}

/// Largest index <= `idx` that is a char boundary of `s`.
fn floor_boundary(s: &str, mut idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    while !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// Renders `changes` (the `detect_changes` tool output, wrapped or not) as markdown that
/// never exceeds `max_bytes` and always starts with [`REPORT_MARKER`].
pub fn render_report(changes: &Value, max_bytes: usize) -> String {
    let result = changes.get("result").unwrap_or(changes);
    let files = array(result, "changed_files");
    let nodes = array(result, "changed_nodes");
    let mut out = format!("{REPORT_MARKER}\n## CodeGraph PR report\n\n");
    if files.is_empty() && nodes.is_empty() {
        out.push_str("no changes analyzed.\n");
        return cap(out, max_bytes);
    }
    let score = result["risk_score"].as_f64().unwrap_or(0.0);
    let _ = writeln!(
        out,
        "**Risk: {} ({score:.2})** - {} changed file(s), {} changed function(s)\n",
        risk_level(score).label(),
        files.len(),
        nodes.len()
    );
    if let Some(base) = result["base_ref"].as_str() {
        let _ = writeln!(out, "Base: {}\n", code(base));
    }

    let priorities = array(result, "review_priorities");
    if !priorities.is_empty() {
        out.push_str(
            "### Review priorities\n\n| # | Function | File | Risk |\n|---|---|---|---|\n",
        );
        for (i, node) in priorities.iter().take(PRIORITY_ROWS).enumerate() {
            let _ = writeln!(
                out,
                "| {} | {} | {} | {:.2} |",
                i + 1,
                code(node["name"].as_str().unwrap_or("?")),
                code(&display_path(node, files)),
                node["risk_score"].as_f64().unwrap_or(0.0)
            );
        }
        out.push('\n');
    }

    let file_lines = files.iter().map(|f| {
        let ranges: Vec<String> = array(f, "lines")
            .iter()
            .filter_map(|r| Some(format!("{}-{}", r[0].as_i64()?, r[1].as_i64()?)))
            .collect();
        format!(
            "- {} (lines {})",
            code(f["path"].as_str().unwrap_or("?")),
            ranges.join(", ")
        )
    });
    out.push_str(&details("Changed files", files.len(), file_lines));

    let node_lines = nodes.iter().map(|n| {
        format!(
            "- {} ({}) in {} - risk {:.2}",
            code(n["name"].as_str().unwrap_or("?")),
            code(n["kind"].as_str().unwrap_or("?")),
            code(&display_path(n, files)),
            n["risk_score"].as_f64().unwrap_or(0.0)
        )
    });
    out.push_str(&details("Changed functions", nodes.len(), node_lines));
    cap(out, max_bytes)
}

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().map_or(&[], Vec::as_slice)
}

/// Renders an untrusted value (names, paths, refs come from repo contents) as an inline
/// code span that cannot break out: control characters become spaces, `<`, `>` and `|`
/// are swapped for look-alike characters (so no raw HTML and no extra table columns),
/// and the fence is longer than any backtick run in the value.
fn code(value: &str) -> String {
    let clean: String = value
        .chars()
        .map(|c| match c {
            '<' => '\u{2039}',
            '>' => '\u{203a}',
            '|' => '\u{00a6}',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let (mut longest, mut run) = (0, 0);
    for c in clean.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat(longest + 1);
    let pad = if clean.starts_with('`') || clean.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{clean}{pad}{fence}")
}

/// Node paths are absolute; prefer the repo-relative path from `changed_files`.
fn display_path(node: &Value, files: &[Value]) -> String {
    let full = node["location"]["file_path"].as_str().unwrap_or("?");
    files
        .iter()
        .filter_map(|f| f["path"].as_str())
        .find(|p| {
            full == *p
                || full
                    .strip_suffix(*p)
                    .is_some_and(|head| head.ends_with('/') || head.ends_with('\\'))
        })
        .unwrap_or(full)
        .to_string()
}

/// A `<details>` block holding `lines`, cut at [`MAX_DETAILS_BYTES`] with a note.
fn details(title: &str, count: usize, lines: impl Iterator<Item = String>) -> String {
    if count == 0 {
        return String::new();
    }
    let mut body = String::new();
    let mut shown = 0;
    for line in lines {
        if body.len() + line.len() + 1 > MAX_DETAILS_BYTES {
            break;
        }
        body.push_str(&line);
        body.push('\n');
        shown += 1;
    }
    if shown < count {
        let _ = writeln!(body, "\n_{} more entries truncated_", count - shown);
    }
    format!("<details><summary>{title} ({count})</summary>\n\n{body}\n</details>\n\n")
}

/// Cuts `text` to `max_bytes` on a char boundary, appending a note about what was cut.
/// An unclosed `<details>` is closed in the note. Only inline code spans are generated,
/// so no fenced block can be left open.
fn cap(text: String, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text;
    }
    let note = |cut: usize, close: bool| {
        let tail = if close { "\n</details>" } else { "" };
        format!("\n\n_report truncated: {cut} bytes cut_{tail}\n")
    };
    // The note's length depends on the digit count of the cut size; reserve for the widest.
    let reserve = note(text.len(), true).len();
    let keep = floor_boundary(&text, max_bytes.saturating_sub(reserve));
    let kept = &text[..keep];
    let open = kept.matches("<details>").count() > kept.matches("</details>").count();
    let mut out = kept.to_string();
    out.push_str(&note(text.len() - keep, open));
    if out.len() > max_bytes {
        // Budget too small even for the note: hard cut keeps the size contract.
        out.truncate(floor_boundary(&out, max_bytes));
    }
    out
}

/// Runs `detect_changes` through the shared tool path and writes the report.
/// Returns the process exit code: 0, [`EXIT_RISK_GATE`] or [`EXIT_ANALYSIS_FAILED`].
///
/// # Safety
/// Must be called from the single-threaded process entry point (see `tool_cli::run`).
pub unsafe fn run(
    base: &str,
    fail_on: FailOn,
    output: Option<&Path>,
    target: &ToolTarget,
    verbose: bool,
    config_path: Option<&Path>,
) -> i32 {
    match unsafe { analyze(base, fail_on, output, target, verbose, config_path) } {
        Ok(code) => code,
        Err(error) => {
            eprintln!("pr-report failed: {error:#}");
            EXIT_ANALYSIS_FAILED
        }
    }
}

unsafe fn analyze(
    base: &str,
    fail_on: FailOn,
    output: Option<&Path>,
    target: &ToolTarget,
    verbose: bool,
    config_path: Option<&Path>,
) -> Result<i32> {
    // The executor would create an empty graph and report "no changes"; never let a
    // missing or empty graph produce a reassuring report.
    // `run_to_value` changes the working directory to the project, so resolve every relative
    // path against the invoking directory once, before the first call.
    let project =
        std::path::absolute(target.project.clone().unwrap_or_else(|| PathBuf::from(".")))?;
    let config_abs: Option<PathBuf> = config_path.map(std::path::absolute).transpose()?;
    let config_path = config_abs.as_deref();
    let mut target = target.clone();
    target.project = Some(project.clone());
    let target = &target;
    let db = project.join(".code-review-graph").join("graph.db");
    if !db.is_file() {
        bail!("no graph at {}; run `codegraph build` first", db.display());
    }
    let params = json!({"base_ref": base, "include_churn": true});
    // `output` is relative to the invoking directory, not the selected project.
    let output: Option<PathBuf> = output.map(std::path::absolute).transpose()?;
    // SAFETY: forwarded from the caller's single-threaded entry point guarantee.
    let stats = unsafe {
        tool_cli::run_to_value("list_graph_stats", json!({}), target, verbose, config_path)
    }?;
    if stats["result"]["total_nodes"].as_u64().unwrap_or(0) == 0 {
        bail!(
            "graph at {} has no nodes; run `codegraph build` first",
            db.display()
        );
    }
    // SAFETY: forwarded from the caller's single-threaded entry point guarantee.
    let changes =
        unsafe { tool_cli::run_to_value("detect_changes", params, target, verbose, config_path) }?;
    let Some(score) = changes["result"]["risk_score"].as_f64() else {
        bail!("detect_changes returned no result.risk_score");
    };
    let report = render_report(&changes, MAX_REPORT_BYTES);
    match output {
        Some(path) => write_report(&path, &report)?,
        None => print!("{report}"),
    }
    Ok(gate_exit_code(score, fail_on))
}

/// Writes the report to `path`.
pub fn write_report(path: &Path, report: &str) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Cannot create {}", parent.display()))?;
    }
    std::fs::write(path, report).with_context(|| format!("Cannot write {}", path.display()))
}
