#!/usr/bin/env bash
# Build, run workflow e2e scenarios against tests/data, then instrumented tests + line coverage.
# Pass-through: ./test.sh -q  and  ./test.sh -- --nocapture  work as for cargo test / llvm-cov.
#
# Coverage (after tests): needs cargo-llvm-cov and rustup component llvm-tools-preview.
#   cargo install cargo-llvm-cov
#   rustup component add llvm-tools-preview
# Set GRAPH_RUN_TEST_NO_COVERAGE=1 to skip coverage and run plain cargo test.
#
# Stops on the first failure: ERR trap + inherit_errexit; a failed `cargo llvm-cov test` exits
# immediately (no automatic re-run of `cargo test`).
set -Eeuo pipefail
# Propagate errexit into $(...) subshells (bash 4.4+).
shopt -s inherit_errexit 2>/dev/null || true

# Stop the whole script on the first unexpected command failure (not disabled in `if` tests).
on_err() {
  local ec=$?
  echo "test.sh: FAILED (exit $ec): ${BASH_COMMAND}" >&2
  exit "$ec"
}
trap on_err ERR

# Expected-failure e2e blocks run with errexit off; disable ERR there so a nonzero graph_run exit
# does not fire the trap.
strict_off_expected_failure() {
  trap - ERR
  set +e
  set +o pipefail
}

strict_on() {
  set -e
  set -o pipefail
  trap on_err ERR
}

print_line_coverage_summary() {
  local json_path=$1
  echo ""
  echo "Source line coverage (from instrumented cargo test only; e2e graph_run runs above are not included):"
  if command -v jq >/dev/null 2>&1; then
    jq -r '.data[0].files[] | "\(.filename | sub(".*/"; "")): \(.summary.lines.covered)/\(.summary.lines.count) lines (\(.summary.lines.percent | floor)%)"' "$json_path" || exit 1
    jq -r '.data[0].totals.lines | "TOTAL: \(.covered)/\(.count) lines (\(.percent | floor)%)"' "$json_path" || exit 1
  elif command -v python3 >/dev/null 2>&1; then
    python3 - "$json_path" <<'PY' || exit 1
import json, sys
with open(sys.argv[1], encoding="utf-8") as f:
    d = json.load(f)
block = d["data"][0]
for fi in block["files"]:
    s = fi["summary"]["lines"]
    name = fi["filename"].rsplit("/", 1)[-1]
    pct = s["percent"]
    print(f"{name}: {s['covered']}/{s['count']} lines ({pct:.1f}%)")
t = block["totals"]["lines"]
tp = t["percent"]
print(f"TOTAL: {t['covered']}/{t['count']} lines ({tp:.1f}%)")
PY
  else
    echo "(Install jq or python3 for a formatted table; raw JSON at $json_path)"
    cat "$json_path" || exit 1
  fi
}

root=$(cd "$(dirname "$0")" && pwd)
cd "$root"
DATA="$root/tests/data"

# Some environments mis-set CARGO_TARGET_DIR to the binary path; cargo needs a directory.
if [[ -n "${CARGO_TARGET_DIR:-}" && ! -d "$CARGO_TARGET_DIR" ]]; then
  unset CARGO_TARGET_DIR
fi

# Each workflow scenario lives under tests/data/<case>/ (00–03 + workflow TOML).
workflow_case_paths() {
  local case=$1 wf=$2
  local dir="$DATA/$case"
  printf '%s\n' \
    "$dir/00_servers.toml" \
    "$dir/01_shells.toml" \
    "$dir/02_commands.toml" \
    "$dir/03_tasks.toml" \
    "$dir/$wf"
}

echo "== build graph_run =="
cargo build -q --bin graph_run
# Match cargo's output location (some environments set CARGO_TARGET_DIR outside ./target).
BIN="${CARGO_TARGET_DIR:-$root/target}/debug/graph_run"
export GRAPH_RUN_BIN="$BIN"

echo "== demo: parallel_hello_datetime (bash) =="
bash "$root/demos/parallel_hello_datetime/bash/run_demo.sh"

echo "== demo: parallel_hello_datetime (powershell) =="
if command -v pwsh >/dev/null 2>&1; then
  pwsh -NoProfile -ExecutionPolicy Bypass -File "$root/demos/parallel_hello_datetime/powershell/run_demo.ps" || exit 1
elif command -v powershell >/dev/null 2>&1; then
  powershell -NoProfile -ExecutionPolicy Bypass -File "$root/demos/parallel_hello_datetime/powershell/run_demo.ps" || exit 1
else
  echo "(skip PowerShell demo: neither pwsh nor powershell in PATH)"
fi

run_ok() {
  local name=$1 case=$2 wf=$3
  shift 3
  echo "== e2e (expect success): $name =="
  # Extra flags (e.g. -vv) before paths keeps intent clear; options can also follow files.
  local paths
  paths=$(workflow_case_paths "$case" "$wf")
  if ! "$BIN" "$@" $paths; then
    echo "e2e failed (exit $?): $name" >&2
    exit 1
  fi
}

run_ok linear workflow_linear 04_workflow_linear.toml

WS="$root/target/graph_run_sh_workspace"
rm -rf "$WS"
echo "== e2e (expect success): workspace =="
if ! "$BIN" $(workflow_case_paths workflow_linear 04_workflow_linear.toml) --workspace "$WS"; then
  echo "e2e failed (exit $?): workspace" >&2
  exit 1
fi
if [[ ! -d "$WS/tmp" || ! -d "$WS/logs" ]]; then
  echo "workspace missing tmp/ or logs/ under $WS" >&2
  exit 1
fi
log_count=$(find "$WS/logs" -type f | wc -l | tr -d ' ')
if [[ "$log_count" -lt 1 ]]; then
  echo "expected at least one file under $WS/logs" >&2
  exit 1
fi

run_ok loop workflow_loop 04_workflow_loop.toml -vv
run_ok fork_join workflow_fork_join 04_workflow_fork_join.toml
run_ok nested_loops workflow_nested_loops 04_workflow_nested_loops.toml

echo "== e2e (expect failure): cyclic workflow without --allow-endless-loop =="
cycl_err=$(mktemp)
strict_off_expected_failure
"$BIN" $(workflow_case_paths workflow_cyclic 04_workflow.toml) 2>"$cycl_err"
cycl_ec=$?
strict_on
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

echo "== e2e (expect failure): abort node after failed task =="
abort_err=$(mktemp)
strict_off_expected_failure
"$BIN" $(workflow_case_paths workflow_abort 04_workflow_abort.toml) 2>"$abort_err"
abort_ec=$?
strict_on
if [[ "$abort_ec" -eq 0 ]]; then
  echo "expected nonzero exit when workflow reaches abort" >&2
  cat "$abort_err" >&2
  rm -f "$abort_err"
  exit 1
fi
if [[ "$abort_ec" -ne 1 ]]; then
  echo "expected exit code 1, got $abort_ec" >&2
  cat "$abort_err" >&2
  rm -f "$abort_err"
  exit 1
fi
if ! grep -q "abort" "$abort_err" || ! grep -q "failure" "$abort_err"; then
  echo "unexpected stderr (want abort + failure):" >&2
  cat "$abort_err" >&2
  rm -f "$abort_err"
  exit 1
fi
rm -f "$abort_err"

if [[ "${GRAPH_RUN_TEST_NO_COVERAGE:-}" == 1 ]]; then
  echo "== cargo test (GRAPH_RUN_TEST_NO_COVERAGE=1) =="
  trap - ERR
  exec cargo test "$@"
fi

if command -v cargo >/dev/null 2>&1 && cargo llvm-cov --version >/dev/null 2>&1; then
  echo "== cargo llvm-cov test (line coverage summary follows) =="
  if command -v rustup >/dev/null 2>&1; then
    rustup component add llvm-tools-preview >/dev/null 2>&1 || true
  fi
  cov_json=$(mktemp)
  trap 'rm -f "${cov_json:-}"' EXIT
  # Do not pass -q here: callers often use ./test.sh -q, which would duplicate -quiet.
  # Any test failure or llvm-cov failure must stop the script (no second cargo test run).
  trap - ERR
  if ! cargo llvm-cov test --json --summary-only --output-path "$cov_json" "$@"; then
    trap - EXIT
    rm -f "$cov_json"
    echo "cargo llvm-cov test failed; fix failing tests or run: cargo test $*" >&2
    exit 1
  fi
  trap on_err ERR
  print_line_coverage_summary "$cov_json"
  trap - EXIT
  rm -f "$cov_json"
else
  echo "== cargo test =="
  echo "Tip: for a line-coverage summary after tests, install:"
  echo "  cargo install cargo-llvm-cov"
  echo "  rustup component add llvm-tools-preview"
  trap - ERR
  exec cargo test "$@"
fi
