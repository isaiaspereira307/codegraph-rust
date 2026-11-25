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
  echo "NO YAML PARSER: install python3-yaml (apt install python3-yaml) or ruby." >&2
  if [ "${ALLOW_NO_YAML_PARSER:-0}" != "1" ]; then
    echo "Set ALLOW_NO_YAML_PARSER=1 to skip the parse check." >&2
    fail=1
  fi
fi

for f in "${files[@]}"; do
  case "$parser" in
    python) err=$(python3 -c 'import sys,yaml; yaml.safe_load(open(sys.argv[1]))' "$f" 2>&1) \
      || { echo "PARSE ERROR: $f: $(echo "$err" | tail -n 1)"; fail=1; } ;;
    ruby) err=$(ruby -ryaml -e 'YAML.load_file(ARGV[0])' "$f" 2>&1) \
      || { echo "PARSE ERROR: $f: $(echo "$err" | head -n 1)"; fail=1; } ;;
  esac
done

# (a2) structural check without a parser: every job under `jobs:` needs runs-on
for f in "${files[@]}"; do
  case "$f" in */action.yml|action.yml) continue ;; esac
  out=$(awk '
    function check() {
      if (job != "" && !has) print FILENAME ": job \"" job "\" has no runs-on"
      if (job != "" && co > 1) print FILENAME ": job \"" job "\" checks out " co " times (lost job header?)"
    }
    /^jobs:/ { injobs=1; next }
    injobs && /^[^ #\t]/ { check(); job=""; injobs=0 }
    injobs && /^  [A-Za-z0-9_-]+:[ \t]*$/ { check(); job=$1; sub(/:$/,"",job); has=0; co=0; next }
    injobs && /^    runs-on:/ { has=1 }
    injobs && /^    uses:/ { has=1 }
    injobs && /uses:[ \t]*actions\/checkout/ { co++ }
    END { check() }
  ' "$f")
  if [ -n "$out" ]; then echo "STRUCTURE: $out"; fail=1; fi
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
