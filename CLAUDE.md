# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

CodeGraph indexes a codebase into a SQLite knowledge graph (the code-review-graph v15 schema) (AST nodes, edges) and serves it to AI clients over MCP as 27 deterministic graph tools plus `build_or_update_graph`: lookup/search (`find_nodes_by_name`, `search_nodes`, `query_graph`, `list_graph_stats`), traversal (dependencies/reverse dependencies, `traverse_graph`, call chains, cycles, coupling), review (`get_impact_radius`, `detect_changes`, `get_review_context`, `get_minimal_context`, with CRG `risk_score`/`risk_signals`/`review_priorities` and optional churn), flows (`list_flows`, `get_flow`, `get_affected_flows`), communities (`list_communities`, `get_community`, `get_architecture_overview`) and insights (`get_hub_nodes`, `get_bridge_nodes`, `get_knowledge_gaps`, `get_surprising_connections`, `get_suggested_questions`, complexity hotspots, large functions). `codegraph tool list` is the authoritative list. No LLM or embedding provider is involved. It is a Rust workspace of 8 crates under `crates/`; the single shipped binary is `codegraph`, built from `crates/codegraph-mcp-server/src/bin/codegraph.rs`.

This fork is being turned into a Rust version of code-review-graph. Phase 0 removed the LLM agent (`codegraph-mcp-rig`), embeddings/reranking (`codegraph-vector`) and the unused `codegraph-zerocopy`/`codegraph-concurrent` crates.

## Commands

**Workspace gotcha:** `default-members = ["crates/codegraph-core"]`, so a bare `cargo build` / `cargo test` / `cargo clippy` only touches `codegraph-core`. Always pass `--workspace` or `-p <crate>`.

```bash
# Fast hygiene pass
cargo check --workspace

# Build the binary (default features = ["daemon"]; `full` adds `server-http`)
cargo build --release -p codegraph-mcp-server --bin codegraph --features full
# What the install scripts do (install-codegraph*.sh):
cargo install --path crates/codegraph-mcp-server --bin codegraph --all-features --force

# Tests
cargo test --workspace                      # or: make test
cargo test -p codegraph-mcp                 # one crate
cargo test -p codegraph-mcp <test_name>     # one test by name filter
cargo test -p codegraph-graph --test sqlite_schema_test     # one integration-test file

# Format / lint
cargo fmt --all                             # rustfmt.toml: max_width = 100
cargo clippy --workspace --all-targets --all-features -- -D warnings   # what CI runs
```

- `make lint` is effectively a no-op (clippy on `codegraph-core` only, with `-A clippy::all -A warnings`). Use the CI form above.
- On macOS, `make build-llvm` / `make test-llvm` link with `lld` for faster builds (requires `ld64.lld` on PATH, e.g. `brew install llvm`).
- Release profile is `lto = "fat"`, `codegen-units = 1`: release builds are slow. Use dev builds (or `--profile fast-dev`) while iterating.

### Stale references to ignore

- Makefile targets referring to `codegraph-api`, `high_perf_test/`, or `scripts/` point at things that no longer exist.
- `docs/TESTING.md` uses feature names (`onnx`, `ollama`, `cloud-jina`, ...) and a `test_mcp_tools.py` that don't match the current tree.
- The feature flags are on `codegraph-mcp-server`: `daemon` (default), `server-http`, `full`.
- The indexer still carries the always-off inference stages (`policy.rs`, `embeddings`/`inference_*` stats fields, `CODEGRAPH_EMBEDDING_POLICY`). Old `[embedding]`/`[rerank]` TOML sections are ignored; from `[llm]` only `context_window` is read.
- The guides listed under "Legacy" in `docs/README.md` (clean install, agentic CLI, AI providers, prompt tiers, testing) still describe the removed agentic tools and embedding providers. The installation and usage guides were deleted; the root `README.md`, `docs/GITHUB_ACTION.md` and `install-codegraph.sh` are current.

### Tests that need external services

There are no `#[ignore]` tests; DB-dependent tests are gated by env vars and silently skip (pass) otherwise:

- None currently; storage tests use temporary SQLite files.

### Running locally

```bash
codegraph db-check                      # opens the store (creating the v15 schema if new) and prints counts
codegraph init /path/to/project         # choose project hooks, merge agent instructions, then index recursively
codegraph init . --hooks none --no-index # instruction setup only; no providers or database
codegraph init . --hooks both --mcp      # also register the MCP server (.mcp.json, .codex/config.toml)
codegraph doctor .                       # read-only health check (exit 1 on any fail)
codegraph uninstall .                    # remove hooks, instruction blocks, skills, MCP entries (--purge-index also deletes the graph)
codegraph index /path/to/project -r -l rust,typescript --index-tier fast|balanced|full
codegraph build . [--skip-flows|--skip-postprocess]   # full rebuild (update: changed files only)
codegraph start stdio --watch           # MCP server over stdio, with re-index-on-change daemon
```

Storage is one SQLite file per repository at `<project>/.code-review-graph/graph.db` (`SqliteGraphStore` in `crates/codegraph-graph/src/sqlite_store/`), the same path and v15 schema code-review-graph (Python) uses, so either tool can read the other's index. WAL mode: many readers, one writer. Writers take an exclusive `flock` on `.code-review-graph/write.lock` (shared with code-review-graph), so `codegraph start` can keep serving while `codegraph index` runs. Rust-only state lives in `cg_*` tables; kind mappings and losses are in `docs/architecture/storage-sqlite.md`.

Config resolution (`crates/codegraph-core/src/config_manager.rs`): `./.codegraph.toml`, then `~/.codegraph/config.toml`, overridden by `.env` (cwd, then `~/.codegraph.env`) and `CODEGRAPH_*` env vars. See `.env.example` and `config/example.toml`. The loader ignores unknown TOML keys silently; `[performance]` and `[logging]` are parsed but have little effect. `ConfigManager::agent_context_window()` (`CODEGRAPH_CONTEXT_WINDOW`, then `[llm] context_window`) sizes graph tool results.

Project init runs before application configuration or the async runtime. `project_init.rs` offers Claude/Codex/both/none, reuses existing CodeGraph hooks, validates both guide files and selected settings, then merges owned instruction blocks. `agent_hooks.rs` preserves unrelated project settings and reuses Codex inline TOML hooks without rewriting TOML. `--no-index` stops after setup; otherwise init loads the selected project's `.env` and configuration and uses the normal index command. Explicit `--config` paths stay relative to the invoking directory. This is separate from global `codegraph config init`; no user-level hooks are created. Test with `cargo test -p codegraph-mcp-server --lib` and `cargo test -p codegraph-mcp-server --test project_init_integration --test tool_cli_integration`.

### Distribution

Releases are built by `.github/workflows/release.yml` (tag `v*`, or `workflow_dispatch` as a dry run) for four targets (Linux/macOS, x86_64/aarch64; Windows is not supported until `mmap.rs` is ported to `windows_sys` and `nix` is gated to unix) and published with `.sha256` files and `SHA256SUMS`; the tag must equal `codegraph --version`. `scripts/release/` packages assets and renders the Homebrew formula (pushed to the tap by the `homebrew` job). `install.sh` is the checksum-verified prebuilt installer (Linux/macOS), distinct from the source-install `install-codegraph.sh`. `action.yml` is a composite Action that runs `codegraph pr-report` (exit 0 ok, 1 analysis failed, 3 risk gate hit); see `docs/GITHUB_ACTION.md`. Shell tests: `scripts/**/test-*.sh`; workflow lint: `scripts/ci/check-workflows.sh`. Release steps: `CONTRIBUTING.md`.

## Architecture

### Crate layering

Dependencies point downward; do not add upward imports (they create cycles):

- `codegraph-core`: types, config, `CodeNode`. Everything depends on it.
- `codegraph-parser` (tree-sitter) and `codegraph-graph` (SQLite storage + petgraph queries) depend only on core.
- `codegraph-mcp-core`: shared MCP types, on core only. `codegraph-mcp-tools` (graph tool schemas + `GraphToolExecutor`) adds graph.
- `codegraph-mcp` (indexer + analyzers) sits on parser/graph/mcp-core.
- `codegraph-mcp-daemon` (file watching) depends on `codegraph-mcp`.
- `codegraph-mcp-server` (CLI, stdio/HTTP transports, MCP tool entrypoints) is the only crate that depends on everything.

Provider and backend code is heavily `#[cfg(feature = ...)]`-gated, so a change can compile under default features and break under `--all-features` (or vice versa). Check with the features that actually enable the code you touched.

### Flow 1: indexing (`codegraph index`)

Inference (embeddings and semantic resolution) has been removed; `InferencePolicies` in `crates/codegraph-mcp/src/policy.rs` are always `off`, and setting `CODEGRAPH_EMBEDDING_POLICY` or `CODEGRAPH_SEMANTIC_RESOLUTION` to anything else is an error.

`bin/codegraph.rs` `handle_index` → `ProjectIndexer::index_project` in `crates/codegraph-mcp/src/indexer.rs`:

1. Collect/prune files and capture immutable, hashed source snapshots with bounded retained bytes and temporary spill files. Fingerprint sources, build/doc inputs and output policy. All entry points (full, single-file, delete and watch) use complete-project reconciliation. `--force` ignores the prior catalog; preparation does not wipe live records.
2. Reuse versioned compressed AST artifacts; pooled tree-sitter parsers honor worker limits, cancellation and early tier gating. FastML state is file-local. Cargo metadata preparation overlaps parsing.
3. Run cached analyzer stages in `crates/codegraph-mcp/src/analyzers/`: Cargo build context, shared/versioned LSP sessions, rustdoc/API enrichment, module linking, scope-aware AST dataflow, docs/contracts and architecture. Optional validated SCIP imports substitute compiler definitions/references for LSP. Preserve direct target IDs and ambiguity.
4. Upsert only changed nodes/file metadata and new stable edges through bounded durable writers. Resolve exact/contextual/normalized targets, then indexed lexical candidates. Persist method provenance.
5. Flush acknowledged writes, remove stale project-scoped records and revalidate final inputs. Persist the project ready marker last.
6. `finalize` refreshes FTS and then runs the post-process (`crates/codegraph-graph/src/analysis/`): traces `flows`/`flow_memberships` and detects Leiden `communities` (`nodes.community_id`, `metadata.community_detector`), rebuilt from scratch each time. `--skip-flows` (level `minimal`) or `--skip-postprocess` (`none`) on `index`/`build`/`update`, or `CODEGRAPH_POSTPROCESS=full|minimal|none` (validated when the indexer starts), skip it and keep the old rows. A failing post-process only warns (`metadata.postprocess_level = failed`). `metadata.postprocess_level` records the last level; an unchanged-source `full` run still finalizes when it is not `full`. The opt-in git post-commit hook runs `update --skip-flows`, as CRG's does.

The index tier controls analyzers and early extraction: `fast` (default; core AST edges), `balanced` (adds build context, LSP symbols, enrichment, module linking, docs), `full` (all analyzers/definitions/references). Enabled-feature inference defaults to synchronous independently of tier. `CODEGRAPH_ANALYZERS=0` disables analyzers; enabled LSP tiers require installed tools unless tool checks are explicitly relaxed or SCIP substitutes LSP. Rust preflight probes `rust-analyzer --version` from the target project before parsing to reject broken rustup shims; install its component with `rustup component add rust-analyzer` in that project. Language servers use the project as their working directory, retain a bounded stderr tail for errors, and replace stopped pooled sessions. Sessions pipeline/deduplicate requests with a 30-second request timeout and `CODEGRAPH_LSP_REQUESTS` outstanding-request limit (default 32). Symbol/definition reads retry only `ContentModified` (`-32801`), up to five times with fresh IDs and backoff within that same deadline. Changed document versions, cancellation, other errors and exhausted retries fail with method/file diagnostics; do not turn them into empty LSP results. Details: `docs/architecture/indexing-analyzers.md`, `docs/specifications/indexing_tiers.spec.md`.

Performance and correctness controls are documented in `docs/indexing-performance-work.md`; use `docs/indexing-benchmarks.md` for reproducible cold/warm/edit/rename/delete/doc/manifest/tier scenarios and same-model retrieval probes. Preserve compatible SHA node IDs, canonical cache fingerprints, source spans, ambiguity and completion barriers. Never silently change the selected model/backend or mix query/document tasks in caches. Reduced precision, candidate caps, alternate splitters and compiler imports stay opt-in until relevant quality measurements support adoption.

Targeted offline checks include reconciliation/watch tests, `cargo test -p codegraph-graph --test sqlite_write_test --test sqlite_reconcile_test`, parser unit tests and the Python benchmark regressions. Strict workspace Clippy currently fails on existing `codegraph-core` diagnostics; do not describe non-strict targeted linting as a strict workspace pass. Debug synthetic measurements do not establish production speedups or live-model quality.

### Flow 2: graph tools (MCP and CLI)

`crates/codegraph-mcp-server/src/official_server.rs` exposes one rmcp `#[tool]` per graph tool, each with a typed parameter struct. The project store is opened lazily on the first call (`open_executor`) and shared through a `OnceCell`; every call goes through `GraphToolExecutor::execute` in `codegraph-mcp-tools`, which validates the tool name, caches results (LRU) and shortens long `content` fields. The executor queries `GraphQuery` (`crates/codegraph-graph/src/graph_query/`), an in-memory `petgraph` snapshot of the SQLite store that reloads when `PRAGMA data_version` changes.

`codegraph tool run <name> --params '<json>'` (`tool_cli.rs`) runs the same path without an MCP transport; `codegraph tool list` prints the schemas and `codegraph tool instructions` the agent guidance from `src/prompts/cli_instructions.md`. Adding a tool needs: the schema in `graph_tool_schemas.rs`, a dispatch arm and `execute_*` method in `graph_tool_executor.rs`, and a `#[tool]` method in `official_server.rs` (the `every_graph_tool_is_exposed_over_mcp` test enforces the last step).

### Schema

`crates/codegraph-graph/src/sqlite_store/schema_v15.sql` is copied verbatim from a database created by code-review-graph v15; do not edit it by hand. `schema_cg.sql` holds the Rust-only `cg_*` tables. Opening a database with another `schema_version` is an error (no migrations are ported). Run `cargo test -p codegraph-graph` after touching either file. A new graph tool needs: a query in `graph_query/`, a schema + executor entry in `codegraph-mcp-tools`, and an MCP `#[tool]` in `official_server.rs`.

## Conventions

- About half the source files start with two `// ABOUTME:` comment lines summarising the file's purpose (`# ABOUTME:` in shell scripts). Follow this on new files.
- Analyzer output must be deterministic, scoped by `project_id`, and carry provenance metadata (analyzer name, confidence).
- Adding a language touches three layers: the enum in `codegraph-core/src/types.rs`, grammar registration in `codegraph-parser/src/language.rs`, and an extractor in `codegraph-parser/src/languages/`; then update `docs/SUPPORTED_LANGUAGES.md`.
- User-facing changes to configuration, providers, or tiers should update `docs/AI_PROVIDERS.md` / `docs/AGENT_PROMPT_TIERS.md`. `CONTRIBUTING.md` has the full "what to change where" map.
- `rules-for-claude-code/codegraph_rule.md` is guidance for *consumers* of the CodeGraph MCP server, not for developing this repo.

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
