# CodeGraph documentation

CodeGraph is a local SQLite code graph served to AI agents over a CLI and MCP; see the
[project README](../README.md) for install and usage.

## Current

- [Supported languages](SUPPORTED_LANGUAGES.md)
- [Storage: SQLite schema and kind mappings](architecture/storage-sqlite.md)
- [Indexing analyzers and tiers](architecture/indexing-analyzers.md)
- [Parity with code-review-graph](parity/baseline.md) and [change analysis](parity/changes.md)
- [Indexing performance](indexing-performance-work.md) and [benchmarks](indexing-benchmarks.md)
- [Debug logging](DEBUG_LOGGING.md)
- [GitHub Action: risk-scored PR review](GITHUB_ACTION.md)
- Design specs and implementation plans: [superpowers/specs](superpowers/specs), [superpowers/plans](superpowers/plans)

## Legacy (describe the previous product, pending rewrite)

These guides still mention SurrealDB, embeddings, LLM providers or the removed agentic tools.
Prefer the README and `codegraph --help`. The old installation and usage guides were removed;
the README and [GitHub Action](GITHUB_ACTION.md) cover those topics.

- [Clean install](CLEAN_INSTALL.md), [agentic CLI](AGENTIC_CLI.md)
- [AI providers](AI_PROVIDERS.md), [agent prompt tiers](AGENT_PROMPT_TIERS.md)
- [Testing](TESTING.md)
