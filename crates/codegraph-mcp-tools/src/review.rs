// ABOUTME: Review context for the current git diff: changed nodes ranked by risk, impact radius and snippets.
// ABOUTME: Shared by get_review_context (full output) and get_minimal_context (ids and counts only).

use crate::git_changes::changed_files;
use codegraph_graph::graph_query::types::{NodeLocation, NodeReference};
use codegraph_graph::graph_query::{GraphQuery, ImpactedNode, NodeCoverage};
use serde::Serialize;
use serde_json::{Value as JsonValue, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct ReviewOptions {
    pub base_ref: String,
    pub depth: i32,
    pub max_nodes: usize,
    pub max_snippet_lines: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewNode {
    #[serde(flatten)]
    pub node: NodeReference,
    /// `dependents` + 1 when the node has no covering test; higher runs first.
    pub risk: usize,
    pub dependents: usize,
    pub tests: usize,
    /// First lines of the node's span, cut at `max_snippet_lines`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewContext {
    pub base_ref: String,
    pub changed_files: Vec<String>,
    /// Changed nodes, highest risk first.
    pub changed_nodes: Vec<ReviewNode>,
    pub impacted: Vec<ImpactedNode>,
    pub affected_files: Vec<String>,
    pub tests: Vec<NodeReference>,
    pub truncated: bool,
}

/// Runs the git diff, maps it to nodes, and gathers risk, impact and snippets.
pub async fn build_review(
    graph: &GraphQuery,
    opts: &ReviewOptions,
) -> Result<ReviewContext, String> {
    let root = PathBuf::from(graph.project_id());
    let base = opts.base_ref.clone();
    let files = tokio::task::spawn_blocking({
        let root = root.clone();
        move || changed_files(&root, &base)
    })
    .await
    .map_err(|e| format!("git task failed: {e}"))??;

    let ranges: Vec<(String, Vec<(i32, i32)>)> = files
        .iter()
        .map(|f| (f.path.clone(), f.lines.clone()))
        .collect();
    let changed = graph
        .nodes_at_lines(&ranges)
        .await
        .map_err(|e| format!("{e:#}"))?;
    let ids: Vec<String> = changed.iter().map(|n| n.id.clone()).collect();
    let coverage: HashMap<String, NodeCoverage> = graph
        .node_coverage(&ids)
        .await
        .map_err(|e| format!("{e:#}"))?
        .into_iter()
        .map(|c| (c.id.clone(), c))
        .collect();
    let radius = graph
        .get_impact_radius(&ids, &[], opts.depth, opts.max_nodes)
        .await
        .map_err(|e| format!("{e:#}"))?;

    let mut nodes: Vec<ReviewNode> = changed
        .into_iter()
        .map(|node| {
            let cov = coverage.get(&node.id);
            let dependents = cov.map_or(0, |c| c.dependents);
            let tests = cov.map_or(0, |c| c.tests);
            let snippet = node
                .location
                .as_ref()
                .and_then(|loc| snippet(&root, loc, opts.max_snippet_lines));
            ReviewNode {
                risk: dependents + usize::from(tests == 0),
                dependents,
                tests,
                snippet,
                node,
            }
        })
        .collect();
    nodes.sort_by(|a, b| {
        b.risk
            .cmp(&a.risk)
            .then_with(|| a.node.name.cmp(&b.node.name))
            .then_with(|| a.node.id.cmp(&b.node.id))
    });

    Ok(ReviewContext {
        base_ref: opts.base_ref.clone(),
        changed_files: files.into_iter().map(|f| f.path).collect(),
        changed_nodes: nodes,
        impacted: radius.impacted,
        affected_files: radius.affected_files,
        tests: radius.tests,
        truncated: radius.truncated,
    })
}

/// Compact view: changed ids with risk, file list and counts; no snippets or impact list.
pub fn minimal_view(ctx: &ReviewContext) -> JsonValue {
    json!({
        "base_ref": ctx.base_ref,
        "changed_files": ctx.changed_files,
        "changed_nodes": ctx.changed_nodes.iter().map(|n| json!({
            "id": n.node.id,
            "risk": n.risk,
        })).collect::<Vec<_>>(),
        "affected_files": ctx.affected_files,
        "counts": {
            "changed_nodes": ctx.changed_nodes.len(),
            "impacted_nodes": ctx.impacted.len(),
            "affected_files": ctx.affected_files.len(),
            "tests": ctx.tests.len(),
        },
        "truncated": ctx.truncated,
    })
}

/// The first `max_lines` lines of a node's span, read from disk; `None` if the file is unreadable.
fn snippet(root: &Path, loc: &NodeLocation, max_lines: usize) -> Option<String> {
    let path = Path::new(&loc.file_path);
    let full = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let text = std::fs::read_to_string(full).ok()?;
    let start = loc.start_line?.max(1) as usize;
    let end = loc.end_line.map_or(start, |e| e.max(start as i32) as usize);
    let last = end.min(start + max_lines.max(1) - 1);
    let lines: Vec<&str> = text
        .lines()
        .skip(start - 1)
        .take(last + 1 - start)
        .collect();
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_core::{CodeNode, EdgeType, Language, Location, NodeType};
    use codegraph_graph::CodeEdge;
    use codegraph_graph::sqlite_store::SqliteGraphStore;
    use std::process::Command;

    fn func(n: u128, name: &str, file: &Path, start: u32, end: u32) -> CodeNode {
        let mut node = CodeNode::new(
            name,
            Some(NodeType::Function),
            Some(Language::Python),
            Location {
                file_path: file.to_string_lossy().into_owned(),
                line: start,
                column: 0,
                end_line: Some(end),
                end_column: None,
            },
        );
        node.id = uuid::Uuid::from_u128(n);
        node
    }

    fn calls(e: u128, from: u128, to: u128) -> CodeEdge {
        let mut edge = CodeEdge::new(
            uuid::Uuid::from_u128(from),
            uuid::Uuid::from_u128(to),
            EdgeType::Calls,
        );
        edge.id = uuid::Uuid::from_u128(e);
        edge
    }

    fn git(root: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?} failed");
    }

    #[tokio::test]
    async fn review_ranks_changed_nodes_and_builds_snippets() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-q"]);
        let source = root.join("m.py");
        std::fs::write(
            &source,
            "def a():\n    return 1\n\ndef b():\n    return a()\n",
        )
        .unwrap();
        git(root, &["add", "m.py"]);
        git(
            root,
            &[
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-q",
                "-m",
                "base",
            ],
        );
        // Edit line 2, inside a.
        std::fs::write(
            &source,
            "def a():\n    return 2\n\ndef b():\n    return a()\n",
        )
        .unwrap();

        let store = SqliteGraphStore::open(root).unwrap();
        let test_file = root.join("test_m.py");
        let mut test = func(30, "test_a", &test_file, 1, 2);
        test.node_type = Some(NodeType::Other("Test".to_string()));
        store
            .upsert_nodes_batch(&[
                func(10, "a", &source, 1, 2),
                func(11, "b", &source, 4, 5),
                test,
            ])
            .await
            .unwrap();
        store
            .upsert_edges_batch(&[calls(100, 11, 10), calls(101, 30, 10)])
            .await
            .unwrap();
        store.finalize("full").await.unwrap();
        let graph = GraphQuery::new(store.clone());

        let opts = ReviewOptions {
            base_ref: "HEAD".to_string(),
            depth: 3,
            max_nodes: 50,
            max_snippet_lines: 20,
        };
        let ctx = build_review(&graph, &opts).await.unwrap();

        // Only `a` changed; it has two dependents (b and the test) and one test.
        assert_eq!(ctx.changed_nodes.len(), 1);
        let a = &ctx.changed_nodes[0];
        assert_eq!(a.node.name, "a");
        assert_eq!((a.dependents, a.tests, a.risk), (2, 1, 2));
        assert_eq!(a.snippet.as_deref(), Some("def a():\n    return 2"));
        assert_eq!(ctx.changed_files, vec!["m.py".to_string()]);
        let impacted: Vec<_> = ctx.impacted.iter().map(|n| n.node.name.as_str()).collect();
        assert_eq!(impacted, vec!["b", "test_a"]);
        assert_eq!(ctx.tests.len(), 1);

        // The minimal view keeps counts and ids, and is smaller than the full one.
        let full = serde_json::to_string(&ctx).unwrap();
        let minimal = serde_json::to_string(&minimal_view(&ctx)).unwrap();
        assert!(minimal.len() < full.len());
        assert_eq!(minimal_view(&ctx)["counts"]["impacted_nodes"], 2);
    }
}
