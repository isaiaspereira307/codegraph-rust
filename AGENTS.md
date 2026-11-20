# Repository Guidelines

## Project Structure & Module Organization

CodeGraph is a Rust 2024 workspace (Rust 1.95 or newer) that indexes code into a SQLite knowledge graph (code-review-graph v15 schema) and exposes deterministic graph tools over MCP and the CLI. Source lives in `crates/codegraph-*/src/`: `core` holds shared types/configuration and artifact caching, `parser` captures sources and extracts syntax, `mcp` reconciles indexing and runs analyzers, `graph` handles storage, and `mcp-tools` holds the graph tool schemas and executor. The CLI is `crates/codegraph-mcp-server/src/bin/codegraph.rs`; MCP tools are declared in `crates/codegraph-mcp-server/src/official_server.rs`.

Unit tests sit alongside modules; integration tests use each crate's `tests/`. The SQLite schema lives in `crates/codegraph-graph/src/sqlite_store/`, configuration examples in `config/`, documentation in `docs/`, and images in `docs/assets/`.

## Build, Test, and Development Commands

Bare Cargo commands target only `codegraph-core`; specify `--workspace` or `-p <crate>`.

- `cargo check --workspace`: check all crates without producing binaries.
- `cargo build -p codegraph-mcp-server --bin codegraph --features full`: build the CLI with daemon and HTTP features; add `--release` for optimized builds.
- `./target/debug/codegraph start stdio`: run the built MCP server . The graph lives in `<project>/.code-review-graph/graph.db`, created on first index. Respect `CARGO_TARGET_DIR` when locating binaries.
- `cargo test -p codegraph-mcp`: test the indexing crate; use `cargo test --workspace` for all crates.
- `cargo test -p codegraph-mcp-server --lib` and `cargo test -p codegraph-mcp-server --test project_init_integration --test tool_cli_integration`: verify project setup, hook preservation and CLI contracts without live providers.
- `cargo fmt --all -- --check`: verify formatting; omit `-- --check` to format.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: run CI-style linting. `make lint` checks only core and suppresses warnings.

## Coding Style & Naming Conventions

Follow `rustfmt.toml`: four spaces, no tabs, 100-column width, and Unix newlines. Use `snake_case` for modules/functions, `PascalCase` for types, and `SCREAMING_SNAKE_CASE` for constants. Follow existing `ABOUTME` file comments. Keep analyzer output deterministic, scoped by `project_id`, and annotated with provenance.

## Testing Guidelines

Use `#[test]` and `#[tokio::test]` with descriptive snake_case names. Add regression tests for changed behavior. After schema edits, run `cargo test -p codegraph-graph`; `schema_v15.sql` must stay identical to code-review-graph's.

Service tests can return successfully without executing when configuration is absent. Report these skips. CI collects coverage with `cargo llvm-cov`; no numeric minimum is configured.

## Indexing Invariants & Performance Checks

Full, single-file, delete and watch paths share complete-project reconciliation. Capture immutable source snapshots once, reuse versioned AST/analyzer artifacts and resolve against the full catalog, including unchanged callers. `--force` reconciles without trusting the previous catalog; it does not wipe the project before preparation.

Keep SHA node identities compatible, distinguish real same-line collisions and retain stable occurrence-aware edge IDs. Resolve definitions/exact/contextual and lexical targets before semantic inference; leave ambiguity unresolved and retain resolution provenance. Chunking must preserve Unicode/structural whitespace and enforce the provider's actual tokenizer budget. Cache keys must include source, extraction policy, model/task/revision/tokenizer/runtime identity and relevant build/doc inputs.

Bound parser workers, retained source bytes, and queued writer payloads independently. Database errors and flush acknowledgements must propagate. Reconcile stale project-scoped records and validate final inputs before persisting readiness. Do not mark a failed stage complete.

Rust LSP preflight must verify `rust-analyzer --version` in the target project, including rustup shims; presence on PATH is insufficient. Start language servers in that project directory, preserve a bounded stderr tail in failure reports and replace stopped pooled sessions. Retry only `ContentModified` (`-32801`) for symbol/definition reads, with fresh IDs, unchanged document versions and at most five retries within one 30-second deadline. Propagate cancellation, other errors, changed versions and exhausted retries. Test startup/crash/retry handling with isolated mock servers and `index_cli_integration` without installing toolchains or indexing the working repository.

Useful offline regressions: `cargo test -p codegraph-mcp --lib`, `cargo test -p codegraph-mcp --test reconciliation_test --test daemon_watch`, `cargo test -p codegraph-graph --test sqlite_write_test --test sqlite_reconcile_test`, and `python3 -m unittest discover -s scripts/benchmarks -p 'test_*.py'`. Strict workspace Clippy currently reports existing `codegraph-core` diagnostics; disclose that limitation instead of calling a non-strict run lint-clean.

## Commit & Pull Request Guidelines

Follow the history's Conventional Commit pattern, e.g. `fix(lsp): prevent premature process kill`. Use semantic PR titles and `.github/PULL_REQUEST_TEMPLATE.md`. Explain motivation, link issues, report validation and performance impact, and update affected documentation. Include screenshots when relevant.

## Security & Configuration

Use `.env.example` and `config/example.toml` as references. Keep credentials in ignored local configuration; never commit API keys or database passwords.

`codegraph init [project]` selects project-local Claude/Codex hooks before merging both agent instruction files and indexing. `--hooks claude|codex|both|none` supports scripts; `--no-index` performs provider-independent setup. Keep user-level settings untouched, preserve unrelated hooks/instructions, and retain idempotent managed blocks. See [init and CLI usage](docs/AGENTIC_CLI.md).

<!-- codegraph:begin -->
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
<!-- codegraph:end -->
