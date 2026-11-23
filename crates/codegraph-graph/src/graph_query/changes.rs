// ABOUTME: Maps changed line ranges per file to the graph nodes whose spans they touch.
// ABOUTME: File nodes are skipped; a node matches when any changed range overlaps its line span.

use super::GraphQuery;
use super::types::NodeReference;
use anyhow::Result;

impl GraphQuery {
    /// Nodes (except File nodes) whose `[line_start, line_end]` overlaps a changed range of a
    /// file. A file matches by exact path or by path suffix, so repository-relative paths work
    /// against stored absolute paths. Sorted by name, then id.
    pub async fn nodes_at_lines(
        &self,
        changed: &[(String, Vec<(i32, i32)>)],
    ) -> Result<Vec<NodeReference>> {
        let snap = self.snapshot().await?;
        let mut hits = Vec::new();
        for ix in snap.graph.node_indices() {
            let n = snap.node(ix);
            if n.kind.as_deref() == Some("File") {
                continue;
            }
            let Some(start) = n.start_line else {
                continue;
            };
            let end = n.end_line.unwrap_or(start);
            let touched = changed.iter().any(|(path, ranges)| {
                let same_file = n.file_path == *path || n.file_path.ends_with(&format!("/{path}"));
                same_file && ranges.iter().any(|&(from, to)| from <= end && start <= to)
            });
            if touched {
                hits.push(ix);
            }
        }
        hits.sort_by(|a, b| snap.order_key(*a).cmp(&snap.order_key(*b)));
        Ok(hits.into_iter().map(|ix| snap.reference(ix)).collect())
    }
}
