# CodeGraph configuration

`example.toml` in this directory is the reference configuration file. Copy it to one of:

- `./.codegraph.toml` in a project (used when present), or
- `~/.codegraph/config.toml` for all projects (`codegraph config init` creates this file with
  every default written out).

A `.env` file in the working directory (then `~/.codegraph.env`) and `CODEGRAPH_*` environment
variables override the config file. `../.env.example` lists the environment variables.
For a file named `codegraph.toml`, select it explicitly with
`--config codegraph.toml` or `CODEGRAPH_CONFIG_PATH=codegraph.toml`.

## What the config file controls

| Section | Effect |
| --- | --- |
| `[indexing]` | Default indexing tier (`fast`, `balanced`, `full`). |
| `[daemon]` | File-watcher behaviour for `codegraph start --watch` and `codegraph daemon`. |
| `[llm] context_window` | Context window the graph tool result limits are sized for. `CODEGRAPH_CONTEXT_WINDOW` wins. Other `[llm]` keys are ignored. |

The loader also accepts `[performance]` and `[logging]`. Unknown sections and keys
are ignored silently, including `[embedding]` and `[rerank]` from older releases.

Storage is not configured here. Each project uses `<project>/.code-review-graph/graph.db`,
the same file and schema (v15) as code-review-graph.

## Checking what is in effect

```bash
codegraph config show          # merged configuration
codegraph config show --json
codegraph config validate
```
