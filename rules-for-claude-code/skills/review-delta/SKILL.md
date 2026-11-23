---
name: review-delta
description: Review only what changed since the last review point (a commit or tag the user names). Use for incremental re-review.
---

# review-delta

1. Ask for or take the last reviewed ref (default: the last commit mentioned in the conversation).
2. Refresh the graph: `codegraph update .`.
3. Map only the delta: `codegraph detect-changes --base <ref>`.
4. Impact of the delta only: `codegraph impact --files <comma-separated changed files>`.
5. Review just those nodes and their direct dependents; do not re-review unchanged code.
6. Report findings by file:line and state the ref used as the baseline.
