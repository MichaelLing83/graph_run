#!/usr/bin/env bash
# SFTP file transfer against docker-ssh-test-up.sh. Requires Docker, bash, and port 2222 free.
# From repo root: run scripts/docker-ssh-test-up.sh (or rely on integration test env), then:
#   ./tests/data/test_file_transfer/run_case.sh
set -euo pipefail
CASE_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../_common.sh
source "$CASE_DIR/../_common.sh"
cd "$CASE_DIR" || exit 1

shopt -s nullglob
glob=(1*.toml)
if ((${#glob[@]} == 0)); then
  echo "run_case.sh: no 1*.toml in $CASE_DIR" >&2
  exit 1
fi

sorted=()
while IFS= read -r line; do
  [[ -n "$line" ]] && sorted+=("$line")
done < <(printf '%s\n' "${glob[@]}" | LC_ALL=C sort)

exec "$GRAPH_RUN_BIN" \
  --constants 00_constants.toml \
  "${sorted[@]}" \
  25_workflow.toml \
  --workspace ./.workspace \
  -vvvvv
