#!/usr/bin/env bash
# Build, run workflow e2e scenarios against tests/data, then cargo test.
# Pass-through: ./test.sh -q  and  ./test.sh -- --nocapture  work as for cargo test.
set -euo pipefail

root=$(cd "$(dirname "$0")" && pwd)
cd "$root"
DATA="$root/tests/data"
BIN="$root/target/debug/graph_run"

base_args=(
  --servers "$DATA/00_servers.toml"
  --shells "$DATA/01_shells.toml"
  --commands "$DATA/02_commands.toml"
  --tasks "$DATA/03_tasks.toml"
)

echo "== build graph_run =="
cargo build -q --bin graph_run

run_ok() {
  local name=$1 wf=$2
  shift 2
  echo "== e2e (expect success): $name =="
  "$BIN" "${base_args[@]}" "$@" "$DATA/$wf"
}

run_ok linear 04_workflow_linear.toml

WS="$root/target/graph_run_sh_workspace"
rm -rf "$WS"
echo "== e2e (expect success): workspace =="
"$BIN" "${base_args[@]}" --workspace "$WS" "$DATA/04_workflow_linear.toml"
if [[ ! -d "$WS/tmp" || ! -d "$WS/logs" ]]; then
  echo "workspace missing tmp/ or logs/ under $WS" >&2
  exit 1
fi
log_count=$(find "$WS/logs" -type f | wc -l | tr -d ' ')
if [[ "$log_count" -lt 1 ]]; then
  echo "expected at least one file under $WS/logs" >&2
  exit 1
fi

run_ok loop 04_workflow_loop.toml -vv
run_ok fork_join 04_workflow_fork_join.toml
run_ok nested_loops 04_workflow_nested_loops.toml

echo "== e2e (expect failure): cyclic workflow without --allow-endless-loop =="
cycl_err=$(mktemp)
set +o pipefail
set +e
"$BIN" "${base_args[@]}" "$DATA/04_workflow.toml" 2>"$cycl_err"
cycl_ec=$?
set -e
set -o pipefail
if [[ "$cycl_ec" -eq 0 ]]; then
  echo "expected nonzero exit for cyclic workflow" >&2
  cat "$cycl_err" >&2
  rm -f "$cycl_err"
  exit 1
fi
if ! grep -q "directed cycle" "$cycl_err" || ! grep -q "allow-endless-loop" "$cycl_err"; then
  echo "unexpected stderr (want cycle + flag hint):" >&2
  cat "$cycl_err" >&2
  rm -f "$cycl_err"
  exit 1
fi
rm -f "$cycl_err"

echo "== cargo test =="
exec cargo test "$@"
