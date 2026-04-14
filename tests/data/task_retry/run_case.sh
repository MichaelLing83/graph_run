#!/usr/bin/env bash
# Retry task scenarios. Usage: ./run_case.sh [success|exhaust]  (default: success)
set -euo pipefail
CASE_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../_common.sh
source "$CASE_DIR/../_common.sh"
mode="${1:-success}"
case "$mode" in
success) wf="04_workflow_retry_success.toml" ;;
exhaust) wf="04_workflow_retry_exhaust.toml" ;;
*)
  echo "usage: $0 [success|exhaust]" >&2
  exit 2
  ;;
esac
exec "$GRAPH_RUN_BIN" --workspace "$CASE_DIR/.workspace" \
  "$CASE_DIR/00_servers.toml" \
  "$CASE_DIR/01_shells.toml" \
  "$CASE_DIR/02_commands.toml" \
  "$CASE_DIR/03_tasks.toml" \
  "$CASE_DIR/$wf"
