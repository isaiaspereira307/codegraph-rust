# CodeGraph Usage Guide

CodeGraph serves the code graph of an indexed project. Its tools return deterministic
graph data (node IDs, names, kinds, file locations) instead of file contents, so they
answer structural questions without reading whole files. No language model runs on
the server side.

## Workflow

1. Call `find_nodes_by_name` with a symbol name or fragment to get its `node_id`.
2. Pass that `node_id` to the traversal and metric tools below.
3. Read only the file:line locations the results point to.

## Tool Selection

| Question | Tool |
|----------|------|
| "Where is X defined?" | `find_nodes_by_name` |
| "If I change X, what breaks?" | `get_reverse_dependencies` |
| "What does X need?" | `get_transitive_dependencies` |
| "What does this function end up calling?" | `trace_call_chain` |
| "Are there dependency cycles?" | `detect_circular_dependencies` |
| "How coupled is X?" | `calculate_coupling_metrics` |
| "What are the central components?" | `get_hub_nodes` |
| "Where should review focus?" | `find_complexity_hotspots` |

`edge_type` selects the relationship to follow: `Calls`, `Imports`, `Uses`,
`Extends`, `Implements` or `References`. Depth parameters are clamped server-side.

## Notes

- The index can lag behind uncommitted edits; verify against current source before
  changing code.
- An empty result for a fresh project usually means it has not been indexed yet
  (`codegraph index <path>`), or that `CODEGRAPH_PROJECT_ID` does not match the
  indexed project.
