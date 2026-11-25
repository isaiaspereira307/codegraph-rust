# CodeGraph

CodeGraph indexes a repository into a local SQLite knowledge graph and gives AI coding agents
deterministic answers about it: who calls this, what depends on it, what a change puts at risk.
One Rust binary; no model provider and no database server to run.

It uses the same on-disk graph as [code-review-graph](https://github.com/tirth8205/code-review-graph)
(schema v15, `.code-review-graph/graph.db`), so either tool can read the other's index.

## Why

Agents burn tokens reading whole files to answer structural questions. A graph answers them
exactly and cheaply. Measured on this repository for a 3-commit change: reading the touched files
costs ~376 KB, `get_review_context` ~47 KB and `get_minimal_context` ~4 KB.

## Install

Prebuilt binaries and the Homebrew tap only exist once the first release is published
(no `v*` tag has been cut yet). Until then, build from source.

**Installer script** (Linux and macOS; verifies the release checksum; after the first release):

```bash
curl -fsSL https://raw.githubusercontent.com/isaiaspereira307/codegraph-rust/main/install.sh | sh
```

It installs to `~/.local/bin`. Environment variables: `CODEGRAPH_VERSION` (for example `1.2.3`,
default latest), `CODEGRAPH_INSTALL_DIR`, `CODEGRAPH_RELEASE_BASE_URL`. `bash install.sh --help`
lists them.

**Homebrew** (after the first release; the formula lives in the tap
`isaiaspereira307/homebrew-codegraph`, which must exist and be populated by the release workflow):

```bash
brew install isaiaspereira307/codegraph/codegraph
```

**GitHub Releases** (after the first release): download `codegraph-<version>-<target>.tar.gz`
(Linux and macOS, x86_64 and aarch64) or `codegraph-<version>-x86_64-pc-windows-msvc.zip`
(Windows) from the [releases page](https://github.com/isaiaspereira307/codegraph-rust/releases),
and check it against the matching `.sha256` file or `SHA256SUMS`. There is no installer script for
Windows; unzip and put `codegraph.exe` on your `PATH`.

**From source** (works today; needs a Rust toolchain, the binary does not link OpenSSL):

```bash
git clone https://github.com/isaiaspereira307/codegraph-rust && cd codegraph-rust
./install-codegraph.sh
# or: cargo install --path crates/codegraph-mcp-server --bin codegraph --all-features --force
```

## GitHub Action

The repository ships a composite Action that posts a sticky, risk-scored review comment on pull
requests. It installs the released binary, so it also needs the first release; until then use
`@main`.

```yaml
name: PR Review
on: pull_request
permissions:
  contents: read
  pull-requests: write
jobs:
  review:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - uses: isaiaspereira307/codegraph-rust@main
        with:
          github-token: ${{ secrets.GITHUB_TOKEN }}
          fail-on-risk: none   # or high / critical to fail the job
```

Inputs, fork-PR behaviour and limits: [docs/GITHUB_ACTION.md](docs/GITHUB_ACTION.md). The same
report is available locally with `codegraph pr-report --base origin/main`.

## Quick start

From the project you want to index:

```bash
codegraph init .                # pick Claude Code, Codex, both or none; adds hooks + instructions; builds the graph
codegraph doctor .              # checks binary, graph, hooks, skills and MCP entries
codegraph tool run find_nodes_by_name --params '{"name": "parse_config"}'
```

`init` only touches the project: `.claude/settings.json` and/or `.codex/hooks.json`,
`CLAUDE.md` / `AGENTS.md` (a marked block), and the Claude Code review skills in
`.claude/skills/`. Nothing is written to `~/.claude` or `~/.codex`. Useful flags:
`--hooks claude|codex|both|none`, `--no-index` (setup only), `--mcp` (see below).

Everyday commands:

```bash
codegraph build .               # full rebuild
codegraph update .              # only changed files
codegraph detect-changes        # graph nodes touched by the git diff
codegraph impact --files src/lib.rs   # transitive dependents (or --node-ids a,b)
codegraph search "token refresh"
codegraph graph-status
codegraph uninstall .           # remove what init installed (keeps the graph; --purge-index deletes it)
```

An opt-in git post-commit hook keeps the graph fresh: `codegraph hooks git install`.

## Tools

`codegraph tool list` is the authoritative list; each runs as `codegraph tool run <name> --params '<json>'`
or over MCP.

| Area | Tools |
|---|---|
| Lookup | `find_nodes_by_name`, `search_nodes`, `query_graph`, `list_graph_stats` |
| Traversal | `get_transitive_dependencies`, `get_reverse_dependencies`, `traverse_graph`, `trace_call_chain`, `detect_circular_dependencies`, `calculate_coupling_metrics` |
| Review | `get_impact_radius`, `detect_changes`, `get_review_context`, `get_minimal_context` (with `risk_score`, `risk_signals`, `review_priorities`, optional churn) |
| Flows | `list_flows`, `get_flow`, `get_affected_flows` |
| Communities | `list_communities`, `get_community`, `get_architecture_overview` |
| Insights | `get_hub_nodes`, `get_bridge_nodes`, `get_knowledge_gaps`, `get_surprising_connections`, `get_suggested_questions`, `find_complexity_hotspots`, `find_large_functions` |

`build_or_update_graph` is also exposed over MCP.

## MCP (optional)

Claude Code and Codex use the tools through the CLI by default (the `init` instructions tell them
how). To also register the MCP server in a project:

```bash
codegraph init . --hooks both --mcp --no-index
```

This adds a `codegraph` entry to `.mcp.json` (Claude Code) and a marked
`[mcp_servers.codegraph]` block to `.codex/config.toml` (Codex reads project config only for
trusted projects). Manual equivalent:

```json
{ "mcpServers": { "codegraph": { "command": "codegraph", "args": ["start", "stdio"] } } }
```

```toml
[mcp_servers.codegraph]
command = "codegraph"
args = ["start", "stdio"]
```

`codegraph start stdio --watch` also re-indexes on file changes.

## Storage and the other tool

The graph is one SQLite file per repository, WAL mode: many readers, one writer. Writers take
`.code-review-graph/write.lock`, shared with code-review-graph, so a running server keeps serving
during a rebuild.

code-review-graph 2.3.x writes schema 9, which codegraph refuses to open. If its global hooks run
in your project, create `.code-review-graph/crg-hooks-off` (and make the hooks skip such projects),
or upgrade it to 2.4.0+. `codegraph doctor` flags this. Details and kind mappings:
[docs/architecture/storage-sqlite.md](docs/architecture/storage-sqlite.md).

## Languages and indexing tiers

Tree-sitter extraction for Rust, TypeScript/JavaScript, Python, Go, Java, C/C++, Swift, C#, Ruby
and PHP; see [docs/SUPPORTED_LANGUAGES.md](docs/SUPPORTED_LANGUAGES.md). Graph parity with
code-review-graph is measured per language (Python, TypeScript, Go, Rust and Java first) in
[docs/parity/](docs/parity/).

`codegraph index --index-tier fast|balanced|full` controls extra analysis: `fast` (default) is
core AST edges; `balanced` and `full` add build context and language-server enrichment and need
the language servers installed. Details: [docs/architecture/indexing-analyzers.md](docs/architecture/indexing-analyzers.md).

## Configuration

`./.codegraph.toml`, then `~/.codegraph/config.toml`, overridden by `.env` and `CODEGRAPH_*`
environment variables (see `.env.example` and `config/example.toml`). Most users need none.
`CODEGRAPH_POSTPROCESS=full|minimal|none` controls flow and community analysis after a build.

## Development

```bash
cargo check --workspace
cargo test --workspace      # bare `cargo test` only covers codegraph-core
cargo fmt --all
```

See [CLAUDE.md](CLAUDE.md) for the architecture and [CONTRIBUTING.md](CONTRIBUTING.md) for what
to change where (including how to cut a release). Some guides under `docs/` still describe the
previous product and are pending a rewrite; [docs/README.md](docs/README.md) says which.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
