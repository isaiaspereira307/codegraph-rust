#!/usr/bin/env bash
# ABOUTME: Posts or updates the sticky codegraph PR comment (usage: upsert-comment.sh <comment-file>).
# ABOUTME: Only edits comments this token authored; 403 (fork PRs) becomes a warning, exit 0.
set -uo pipefail

file="${1:?usage: upsert-comment.sh <comment-file>}"
: "${GITHUB_REPOSITORY:?}" "${PR_NUMBER:?}"
marker='<!-- codegraph-report -->'

if [ ! -f "$file" ]; then
  echo "::error::comment file not found: $file"
  exit 1
fi

# gh failure handler: 403 (fork PR / read-only token) is not fatal; the report
# remains available through the `comment-file` output.
soft_fail() { # $1 = captured stderr
  if printf '%s' "$1" | grep -qE 'HTTP 403|Resource not accessible'; then
    echo "::warning::Could not post the codegraph PR comment (token lacks pull-requests: write, e.g. fork PR). The report is still available via the comment-file output."
    exit 0
  fi
  echo "::error::Posting the codegraph PR comment failed: $1"
  exit 1
}

# Only ever edit a comment this Action itself posted: marker at the start of the
# body AND author match. A PAT answers `gh api user` with its login (exact);
# an installation token cannot, so fall back to any Bot author (PR participants
# are Users, never Bots).
author="$(gh api user --jq '.login' 2>/dev/null || true)"
if [ -n "$author" ] && printf '%s' "$author" | grep -qE '^[]A-Za-z0-9_.[-]+$'; then
  author_filter=".user.login == \"${author}\""
else
  author_filter='.user.type == "Bot"'
fi

err="$(mktemp)"
trap 'rm -f "$err"' EXIT

if ! ids="$(gh api "repos/${GITHUB_REPOSITORY}/issues/${PR_NUMBER}/comments" --paginate \
  --jq ".[] | select(${author_filter} and ((.body // \"\") | startswith(\"${marker}\"))) | .id" 2>"$err")"; then
  soft_fail "$(cat "$err")"
fi
comment_id="$(printf '%s\n' "$ids" | head -n 1)"

if [ -n "$comment_id" ]; then
  gh api --method PATCH --silent \
    "repos/${GITHUB_REPOSITORY}/issues/comments/${comment_id}" \
    -F body=@"$file" 2>"$err" || soft_fail "$(cat "$err")"
else
  gh api --method POST --silent \
    "repos/${GITHUB_REPOSITORY}/issues/${PR_NUMBER}/comments" \
    -F body=@"$file" 2>"$err" || soft_fail "$(cat "$err")"
fi
