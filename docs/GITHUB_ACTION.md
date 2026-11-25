# GitHub Action: risk-scored PR review

The composite Action in the repository root (`action.yml`) builds the CodeGraph knowledge graph on
the runner, maps the pull request diff to graph nodes with `codegraph pr-report`, and posts the
result as one sticky comment that is updated on every push. Nothing leaves the runner except the
comment itself.

The Action installs a released `codegraph` binary with `install.sh`, so it needs at least one
published release: `@main` only selects the Action's own code, and `install.sh` still downloads a
released binary, so the Action cannot run before the first release is published. Once a release
exists, `uses: isaiaspereira307/codegraph-rust@main` (or a tag, once one exists) works.

## Example

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
          fail-on-risk: high
```

## Inputs and outputs

| Name | Kind | Default | Meaning |
|---|---|---|---|
| `github-token` | input, required | | Token used to post the comment. The default `GITHUB_TOKEN` works with `pull-requests: write`. |
| `comment` | input | `true` | Post and keep updated a sticky PR comment. Set `false` to only produce the report file. |
| `fail-on-risk` | input | `none` | Fail the job when overall risk reaches `high` or `critical`; `none` never fails on risk. Any other value is rejected. |
| `version` | input | `latest` | Release to install, for example `1.2.3`. |
| `comment-file` | output | | Runner-local path of the rendered markdown report. |

## What the report contains

`codegraph pr-report` renders, from `detect_changes`:

- a header with the overall risk level (Low, High at score 0.70 or more, Critical at 0.85 or
  more), the score, and the number of changed files and functions;
- the base ref;
- a "Review priorities" table (function, file, risk) when the graph ranks any;
- collapsible lists of changed files (with line ranges) and changed functions (with kind and risk).

It does not contain impact-radius or test-gap sections; `detect_changes` does not return that data
today. Long reports are truncated to fit a comment. If nothing could be mapped it says
"no changes analyzed".

## Exit codes of `codegraph pr-report`

`codegraph pr-report --base <ref> [--fail-on-risk none|high|critical] [--output <path>]`:

| Exit | Meaning |
|---|---|
| 0 | Report written; risk gate not reached (or `none`). |
| 1 | The analysis failed. The Action fails the job instead of posting a reassuring comment. |
| 3 | The report was written and the `--fail-on-risk` level was reached. The Action still posts the comment, then fails the job. |

## Permissions and fork pull requests

The job needs `pull-requests: write` to comment. On pull requests from forks, `GITHUB_TOKEN` is
read-only, so posting returns 403; the Action downgrades that to a warning and the report stays
available in the `comment-file` output. To publish fork reports, run the Action with
`comment: false` and post the file from a separate, trusted workflow. The Action only edits comments
it posted itself (identified by a hidden marker and the author).

## Caching

`.code-review-graph` is cached with `actions/cache` keyed on the OS and the lockfiles
(`Cargo.lock`, `package-lock.json`, `uv.lock`, `go.sum`). On a cache hit the Action runs
`codegraph update .` and falls back to a full `codegraph build .` if the restored database is
unusable.

## Limitations

- Linux and macOS runners only; on Windows the Action fails with an explicit error.
- Pull requests should check out with `fetch-depth: 0`: the diff base is `origin/<base branch>`.
  On other events the base is `HEAD~1`, which needs `fetch-depth: 2` at least.
- The graph uses the default `fast` index tier (core AST edges).
- Requires a published release (see above).

The repository's own `.github/workflows/pr-review.yml` runs the local Action and is enabled by the
repository variable `CODEGRAPH_DOGFOOD=true`.
