# codegraph

CodeGraph is a CLI tool available through Bash as the `codegraph` command. Run the
commands below using your Bash or shell execution tool. Check that CodeGraph is
on PATH with `command -v codegraph`.

When CodeGraph is available and the project is indexed, use its graph tools for
structural questions (who calls this, what depends on it, what is risky) before
broad grep/rg searches:

- `codegraph tool run find_nodes_by_name --params '{"name": "<symbol>"}'`
  resolves a symbol to node IDs and file locations.
- `codegraph tool run get_reverse_dependencies --params '{"node_id": "<id>", "edge_type": "Calls"}'`
  shows what a change to that node would affect; check it before editing.
- `codegraph tool run trace_call_chain --params '{"from_node": "<id>"}'`
  follows call flows from a function.
- `codegraph tool run find_complexity_hotspots` and `get_hub_nodes`
  point at risky or central code for review and refactoring.

Run from the indexed project root, or append `--project /path/to/project`; retain
the indexed `--project-id` if one was configured. Read the specific files/lines a
result points to before editing. After changes, verify against current source and
run relevant tests; the index may lag uncommitted work.

If CodeGraph is unavailable, the project is not indexed, or a command fails or returns
insufficient evidence, fall back to targeted source reads and rg/grep. Do not install
CodeGraph or reindex solely to satisfy these instructions. List all tools with
`codegraph tool list`; reload usage details with `codegraph tool instructions`.
