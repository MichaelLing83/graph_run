#!/usr/bin/env bash
# Cyclic success graph: graph_run rejects the workflow (nonzero exit). To experiment with running
# it anyway, invoke graph_run yourself with --allow-endless-loop (it may not terminate).
set -euo pipefail
CASE_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../_common.sh
source "$CASE_DIR/../_common.sh"
exec "$GRAPH_RUN_BIN" --workspace "$CASE_DIR/.workspace" \
  "$CASE_DIR/00_servers.toml" \
  "$CASE_DIR/01_shells.toml" \
  "$CASE_DIR/02_commands.toml" \
  "$CASE_DIR/03_tasks.toml" \
  "$CASE_DIR/04_workflow.toml"
