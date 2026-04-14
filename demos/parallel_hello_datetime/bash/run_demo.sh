#!/usr/bin/env bash
# Run the bash variant of the parallel_hello_datetime demo from the repository root.
# Requires a built graph_run (see README). Uses GRAPH_RUN_BIN if set (e.g. by test.sh).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT"

if [[ -n "${CARGO_TARGET_DIR:-}" && ! -d "$CARGO_TARGET_DIR" ]]; then
  unset CARGO_TARGET_DIR
fi

BIN="${GRAPH_RUN_BIN:-${CARGO_TARGET_DIR:-$ROOT/target}/debug/graph_run}"
if [[ ! -x "$BIN" ]]; then
  echo "graph_run not found or not executable at: $BIN" >&2
  echo "Build with: cargo build --bin graph_run" >&2
  exit 1
fi

DEMO_DIR="$(cd "$(dirname "$0")" && pwd)"
exec "$BIN" --workspace "$DEMO_DIR/.workspace" \
  "$DEMO_DIR/00_servers.toml" \
  "$DEMO_DIR/10_shells.toml" \
  "$DEMO_DIR/20_commands.toml" \
  "$DEMO_DIR/30_tasks.toml" \
  "$DEMO_DIR/40_workflow.toml"
