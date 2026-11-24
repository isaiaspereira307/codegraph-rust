#!/bin/bash
# ABOUTME: Installs the codegraph binary with cargo; no database server or model provider is needed.
# ABOUTME: Run from a checkout of this repository.

set -euo pipefail

command -v cargo >/dev/null 2>&1 || { echo "[ERROR] Rust is required (https://rustup.rs)." >&2; exit 1; }

cargo install --path crates/codegraph-mcp-server --bin codegraph --all-features --force

INSTALL_PATH="${CARGO_HOME:-$HOME/.cargo}/bin"
cat <<EOF2

codegraph installed to ${INSTALL_PATH}

Next steps (from the project you want to index):
  codegraph init .        # choose Claude Code / Codex hooks, add instructions, build the graph
  codegraph doctor .      # check that everything is wired up

Add ${INSTALL_PATH} to your PATH if it is not already there.
EOF2
