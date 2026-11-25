#!/usr/bin/env bash
# ABOUTME: Validates GitHub workflow YAMLs (parse) and rejects references to removed things.
# ABOUTME: Exit 0 when clean, 1 on any violation. Reused by the distribution tasks.
set -euo pipefail

cd "$(dirname "$0")/../.."

fail=0

files=()
for f in .github/workflows/*.yml action.yml; do
  [ -f "$f" ] && files+=("$f")
done

# (a) every YAML parses (python3+PyYAML, else ruby; local validator only)
if python3 -c 'import yaml' 2>/dev/null; then
  parser=python
elif command -v ruby >/dev/null 2>&1; then
  parser=ruby
else
  parser=none
  echo "WARN: neither PyYAML nor ruby available; skipping YAML parse check" >&2
fi

for f in "${files[@]}"; do
  case "$parser" in
    python) python3 -c 'import sys,yaml; yaml.safe_load(open(sys.argv[1]))' "$f" 2>/dev/null \
      || { echo "PARSE ERROR: $f"; fail=1; } ;;
    ruby) ruby -ryaml -e 'YAML.load_file(ARGV[0])' "$f" 2>/dev/null \
      || { echo "PARSE ERROR: $f"; fail=1; } ;;
  esac
done

# (b) forbidden references
targets=(.github)
[ -f action.yml ] && targets+=(action.yml)
if hits=$(grep -rn -E 'codegraph-api|libssl-dev|libfaiss|DOCKER_' "${targets[@]}"); then
  echo "FORBIDDEN REFERENCES:"
  echo "$hits"
  fail=1
fi

exit "$fail"
