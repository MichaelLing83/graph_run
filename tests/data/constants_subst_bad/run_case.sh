#!/usr/bin/env bash
# Unknown ${PLACEHOLDER}; graph_run exits nonzero when loading configs (expected).
set -euo pipefail
CASE_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../_common.sh
source "$CASE_DIR/../_common.sh"
exec "$GRAPH_RUN_BIN" \
  --constants "$CASE_DIR/constants.toml" \
  --workspace "$CASE_DIR/.workspace" \
  "$CASE_DIR/00_servers.toml" \
  "$CASE_DIR/01_shells.toml" \
  "$CASE_DIR/02_commands.toml" \
  "$CASE_DIR/03_tasks.toml" \
  "$CASE_DIR/04_workflow_linear.toml"
