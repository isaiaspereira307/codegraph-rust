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
| Callers, callees, imports, importers, tests of a node | `query_graph` | `pattern`, `node_id` or `name`, `limit` |
| Walk dependencies out, in or both, bounded | `traverse_graph` | `node_id` or `name`, `direction`, `edge_type`, `max_depth`, `max_nodes` |
| What does X depend on? | `get_transitive_dependencies` | `node_id`, `edge_type`, `depth` |
| What breaks if X changes? | `get_reverse_dependencies` | `node_id`, `edge_type`, `depth` |
| Follow calls from a function | `trace_call_chain` | `from_node`, `max_depth` |
| Find dependency cycles | `detect_circular_dependencies` | `edge_type` |
| Coupling of a node | `calculate_coupling_metrics` | `node_id` |
| Central components | `get_hub_nodes` | `min_degree` |
| Risky functions to review | `find_complexity_hotspots` | `min_complexity`, `limit` |

`edge_type` is one of `Calls`, `Imports`, `Uses`, `Extends`, `Implements`, `References`.

Examples:
```sh
codegraph tool run find_nodes_by_name --params '{"name": "parse_file"}'
codegraph tool run get_reverse_dependencies --params '{"node_id": "<id>", "edge_type": "Calls"}'
codegraph tool run find_complexity_hotspots --params '{"limit": 10}'
```

Run from the indexed project root, or pass `--project /path/to/project`. For a custom
indexed identifier, pass `--project-id ID` or retain `CODEGRAPH_PROJECT_ID`. Stdout is
one JSON document; logs go to stderr. Runtime failures emit `{"error": {...}}` and
exit 1; argument errors exit 2. `codegraph tool list` prints every parameter schema.

Follow returned file:line locations with targeted reads. The index can lag behind
uncommitted work, so verify against current source before editing. If the project is
not indexed or a result lacks evidence, use ordinary repository tools. Do not install,
configure or reindex merely because this guidance was injected.

Reload this guidance with `codegraph tool instructions` after context loss.
