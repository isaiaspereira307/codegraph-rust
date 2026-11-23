# CodeGraph graph CLI

CodeGraph is a CLI tool available through Bash as the `codegraph` command. Run the
commands below using your Bash or shell execution tool. Check that CodeGraph is
on PATH with `command -v codegraph`.

CodeGraph answers structural questions about an indexed project from its code graph.
Results are deterministic graph data (node IDs, names, kinds, file locations); no
language model is involved and no provider costs apply.

Start with `find_nodes_by_name` to turn a symbol name into a `node_id`, then pass
that ID to the other tools:

| Need | Tool | Parameters |
| --- | --- | --- |
| Resolve a symbol to node IDs | `find_nodes_by_name` | `name`, `limit` |
| Find code by words (stems, camelCase) | `search_nodes` | `query`, `limit` |
| Find long functions (refactoring candidates) | `find_large_functions` | `min_lines`, `limit` |
| Counts of nodes, edges, files and languages | `list_graph_stats` | none |
| Callers, callees, imports, importers, tests of a node | `query_graph` | `pattern`, `node_id` or `name`, `limit` |
| Walk dependencies out, in or both, bounded | `traverse_graph` | `node_id` or `name`, `direction`, `edge_type`, `max_depth`, `max_nodes` |
| Blast radius of changed nodes or files, with tests | `get_impact_radius` | `node_ids` or `files`, `depth`, `max_nodes` |
| Nodes touched by the git diff against a base | `detect_changes` | `base_ref` (default HEAD) |
| Review context for the diff: risk, impact, tests, snippets | `get_review_context` | `base_ref`, `depth`, `max_nodes`, `max_snippet_lines` |
| Same, compact (ids and counts) | `get_minimal_context` | `base_ref`, `depth`, `max_nodes` |
| What does X depend on? | `get_transitive_dependencies` | `node_id`, `edge_type`, `depth` |
| What breaks if X changes? | `get_reverse_dependencies` | `node_id`, `edge_type`, `depth` |
| Follow calls from a function | `trace_call_chain` | `from_node`, `max_depth` |
| Find dependency cycles | `detect_circular_dependencies` | `edge_type` |
| Coupling of a node | `calculate_coupling_metrics` | `node_id` |
| Central components | `get_hub_nodes` | `top_n`, `detail_level` |
| Chokepoints between code regions | `get_bridge_nodes` | `top_n`, `detail_level` |
| Risky functions to review | `find_complexity_hotspots` | `min_complexity`, `limit` |
| Execution flows from entry points, most critical first | `list_flows` | `sort_by`, `limit`, `kind`, `detail_level` |
| One flow with its steps (and source) | `get_flow` | `flow_id` or `flow_name`, `include_source`, `max_steps` |
| Flows that pass through the changed files | `get_affected_flows` | `changed_files` or `base_ref`, `detail_level`, `max_flows` |
| Code clusters (communities) with size and cohesion | `list_communities` | `sort_by`, `min_size`, `detail_level`, `max_members` |
| One community, optionally with member nodes | `get_community` | `community_id` or `community_name`, `include_members` |
| Communities and the coupling between them | `get_architecture_overview` | `detail_level`, `max_results` |

`edge_type` is one of `Calls`, `Imports`, `Uses`, `Extends`, `Implements`, `References`.

Examples:
```sh
codegraph tool run find_nodes_by_name --params '{"name": "parse_file"}'
codegraph tool run get_reverse_dependencies --params '{"node_id": "<id>", "edge_type": "Calls"}'
codegraph tool run find_complexity_hotspots --params '{"limit": 10}'
```

Shortcuts with the same output as `tool run`: `codegraph search <text>`,
`codegraph query <pattern> --name <symbol>`, `codegraph impact --files a.rs,b.rs`,
`codegraph detect-changes --base <ref>`. Build or refresh the graph with
`codegraph build` (full) or `codegraph update` (changed files only).

Run from the indexed project root, or pass `--project /path/to/project`. For a custom
indexed identifier, pass `--project-id ID` or retain `CODEGRAPH_PROJECT_ID`. Stdout is
one JSON document; logs go to stderr. Runtime failures emit `{"error": {...}}` and
exit 1; argument errors exit 2. `codegraph tool list` prints every parameter schema.

Follow returned file:line locations with targeted reads. The index can lag behind
uncommitted work, so verify against current source before editing. If the project is
not indexed or a result lacks evidence, use ordinary repository tools. Do not install,
configure or reindex merely because this guidance was injected.

Reload this guidance with `codegraph tool instructions` after context loss.
