#!/usr/bin/env bash
# Run graph_run on the loop workflow fixture (-vv matches integration test).
set -euo pipefail
CASE_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../_common.sh
source "$CASE_DIR/../_common.sh"
exec "$GRAPH_RUN_BIN" -vv --workspace "$CASE_DIR/.workspace" \
  "$CASE_DIR/00_servers.toml" \
  "$CASE_DIR/01_shells.toml" \
  "$CASE_DIR/02_commands.toml" \
  "$CASE_DIR/03_tasks.toml" \
  "$CASE_DIR/04_workflow_loop.toml"
