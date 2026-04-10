#!/usr/bin/env bash
# Stop and remove the SSH test container started by scripts/docker-ssh-test-up.sh.
#
# Usage:
#   ./scripts/docker-ssh-test-down.sh
#   CONTAINER_NAME=my_ssh_box ./scripts/docker-ssh-test-down.sh
#
# Environment:
#   CONTAINER_NAME   must match the name used with docker-ssh-test-up.sh (default: graph_run_ssh_test)
#
set -euo pipefail

CONTAINER_NAME="${CONTAINER_NAME:-graph_run_ssh_test}"

if docker inspect "$CONTAINER_NAME" >/dev/null 2>&1; then
  docker rm -f "$CONTAINER_NAME" >/dev/null
  echo "Stopped and removed container: $CONTAINER_NAME" >&2
else
  echo "No container named $CONTAINER_NAME (nothing to do)." >&2
fi
