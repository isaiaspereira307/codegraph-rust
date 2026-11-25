#!/usr/bin/env bash
# ABOUTME: Tests scripts/action/upsert-comment.sh against a fake `gh` that records calls and replays canned JSON.
# ABOUTME: Covers POST when absent, PATCH when ours, ignoring User/non-prefix markers, and 403 as warning.
set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
script="$here/upsert-comment.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

mkdir -p "$work/bin"
# Fake gh: logs argv; `api user` succeeds only with FAKE_USER_LOGIN; the comment
# listing emulates the author/marker jq filter over $FAKE_COMMENTS; FAKE_FAIL=403 fails writes+listing.
cat > "$work/bin/gh" <<'FAKE'
#!/usr/bin/env bash
echo "gh $*" >> "$FAKE_LOG"
[ "$1" = api ] || exit 2
if [ "$2" = user ]; then
  [ -n "${FAKE_USER_LOGIN:-}" ] && { echo "$FAKE_USER_LOGIN"; exit 0; }
  echo "gh: Resource not accessible by integration (HTTP 403)" >&2; exit 1
fi
if [ "${FAKE_FAIL:-}" = 403 ]; then
  echo "gh: Resource not accessible by integration (HTTP 403)" >&2; exit 1
fi
method=GET; jq=""
args=("$@")
for ((i = 0; i < ${#args[@]}; i++)); do
  case "${args[$i]}" in
    --method) method="${args[$((i + 1))]}" ;;
    --jq) jq="${args[$((i + 1))]}" ;;
  esac
done
if [ "$method" = GET ]; then
  python3 -I - "$jq" "${FAKE_COMMENTS:-/dev/null}" <<'PY'
import json, re, sys
jq, path = sys.argv[1], sys.argv[2]
try:
    data = json.load(open(path))
except Exception:
    data = []
m = re.search(r'\.user\.login == \\?"([^"\\]+)\\?"', jq)
marker = re.search(r'startswith\(\\?"(.*?)\\?"\)', jq).group(1)
for c in data:
    if m:
        ok = c["user"]["login"] == m.group(1)
    else:
        ok = c["user"]["type"] == "Bot"
    if ok and c["body"].startswith(marker):
        print(c["id"])
PY
fi
exit 0
FAKE
chmod +x "$work/bin/gh"

printf '<!-- codegraph-report -->\n# Report\n' > "$work/comment.md"

pass=0; failn=0
check() { # name, condition-exit-status
  if [ "$2" -eq 0 ]; then echo "PASS: $1"; pass=$((pass + 1)); else echo "FAIL: $1"; failn=$((failn + 1)); fi
}

run() { # runs the script; sets OUT, RC; FAKE_LOG reset
  : > "$work/log"
  OUT="$(PATH="$work/bin:$PATH" FAKE_LOG="$work/log" GH_TOKEN=x GITHUB_REPOSITORY=o/r PR_NUMBER=7 \
    bash "$script" "$work/comment.md" 2>&1)"
  RC=$?
}
count() { grep -c -- "$1" "$work/log" || true; }

comments() { printf '%s' "$1" > "$work/comments.json"; export FAKE_COMMENTS="$work/comments.json"; }
unset FAKE_FAIL FAKE_USER_LOGIN

# 1. no existing comment -> exactly one POST, body from file
comments '[]'
run
[ "$RC" -eq 0 ] && [ "$(count 'method POST')" -eq 1 ] && [ "$(count 'method PATCH')" -eq 0 ] \
  && grep -q 'repos/o/r/issues/7/comments' "$work/log" && grep -q -- "-F body=@$work/comment.md" "$work/log"
check "no existing comment -> one POST with body from file" $?

# 2. existing Bot comment with marker at start -> one PATCH, no POST
comments '[{"id":111,"user":{"login":"github-actions[bot]","type":"Bot"},"body":"<!-- codegraph-report -->\nold"}]'
run
[ "$RC" -eq 0 ] && [ "$(count 'method PATCH')" -eq 1 ] && [ "$(count 'method POST')" -eq 0 ] \
  && grep -q 'repos/o/r/issues/comments/111' "$work/log" && grep -q -- "-F body=@$work/comment.md" "$work/log"
check "existing Bot comment -> one PATCH on its id, no POST" $?

# 3. marker comment authored by a User is ignored -> POST
comments '[{"id":222,"user":{"login":"mallory","type":"User"},"body":"<!-- codegraph-report -->\nfake"}]'
run
[ "$RC" -eq 0 ] && [ "$(count 'method POST')" -eq 1 ] && [ "$(count 'method PATCH')" -eq 0 ]
check "marker comment by a User is ignored -> POST" $?

# 4. marker merely contained (not at start) is ignored -> POST
comments '[{"id":333,"user":{"login":"github-actions[bot]","type":"Bot"},"body":"hello\n<!-- codegraph-report -->"}]'
run
[ "$RC" -eq 0 ] && [ "$(count 'method POST')" -eq 1 ] && [ "$(count 'method PATCH')" -eq 0 ]
check "marker not at start of body is ignored -> POST" $?

# 5. PAT identity: only that login's comment is adopted
export FAKE_USER_LOGIN=alice
comments '[{"id":444,"user":{"login":"github-actions[bot]","type":"Bot"},"body":"<!-- codegraph-report -->\nx"},{"id":555,"user":{"login":"alice","type":"User"},"body":"<!-- codegraph-report -->\nmine"}]'
run
[ "$RC" -eq 0 ] && [ "$(count 'method PATCH')" -eq 1 ] && grep -q 'issues/comments/555' "$work/log"
check "with gh api user, only that author's comment is patched" $?
unset FAKE_USER_LOGIN

# 6. 403 (fork PR) -> ::warning:: and exit 0
comments '[]'
export FAKE_FAIL=403
run
[ "$RC" -eq 0 ] && printf '%s' "$OUT" | grep -q '::warning::'
check "gh 403 -> ::warning:: and exit 0" $?
unset FAKE_FAIL

# 7. missing comment file -> non-zero
OUT="$(PATH="$work/bin:$PATH" FAKE_LOG="$work/log" GH_TOKEN=x GITHUB_REPOSITORY=o/r PR_NUMBER=7 bash "$script" "$work/nope.md" 2>&1)"
[ $? -ne 0 ]
check "missing comment file fails" $?

echo "passed=$pass failed=$failn"
[ "$failn" -eq 0 ]
