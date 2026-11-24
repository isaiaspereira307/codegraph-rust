---
name: review-changes
description: Review uncommitted or HEAD-relative changes using the CodeGraph graph (changed nodes, blast radius, tests). Use for "review my changes" before commit.
---

# review-changes

1. Refresh the graph: `codegraph update .` (incremental; lock conflicts are safe to retry).
2. Map the diff to graph nodes: `codegraph detect-changes` (against HEAD by default).
3. Get the compact review bundle: `codegraph tool run get_minimal_context --params '{}'`.
   Use `get_review_context` only when snippets are needed.
4. Start with the highest `risk_score` nodes (0-1; the top-level `risk` is high above 0.7,
   medium above 0.4). For each high-risk changed node, check dependents:
   `codegraph impact --node-ids <id>`.
5. Read only the files and lines the graph points to. Verify with the project's tests.
6. Report findings by file:line. Say when the graph may lag uncommitted edits.
