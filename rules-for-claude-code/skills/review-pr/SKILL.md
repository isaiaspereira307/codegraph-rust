---
name: review-pr
description: Review a branch or pull request against its base using CodeGraph impact analysis. Use when asked to review a PR or branch.
---

# review-pr

1. Determine the base (the PR's base branch, or `main`). Check out or fetch the head.
2. Refresh the graph: `codegraph update .`.
3. Map the branch diff to nodes: `codegraph detect-changes --base <base>`.
4. Blast radius of the changed files: `codegraph impact --files "$(git diff --name-only <base>...HEAD | paste -sd, -)"`.
5. Check tests covering changed code: `codegraph query tests_for --name <symbol>`.
6. Read the files the graph points to; report findings by file:line, most severe first.
