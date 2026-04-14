# Shared setup for tests/data/*/run_case.sh (source this file, do not execute).
# Sets GRAPH_RUN_ROOT and GRAPH_RUN_BIN. Honors GRAPH_RUN_BIN and CARGO_TARGET_DIR from the environment.

if [[ -n "${_GRAPH_RUN_TEST_COMMON_LOADED:-}" ]]; then
  return 0
fi

# Resolve this file to an absolute path. When sourced as `source ../_common.sh`,
# BASH_SOURCE[0] is `../_common.sh`; `dirname` is `..` and `cd ..` would use the *current*
# working directory, not the case directory — producing e.g. tests/target/... instead of
# <repo>/target/... . Anchor relative paths to the script that sourced us (BASH_SOURCE[1]).
_common_here="${BASH_SOURCE[0]}"
if [[ "$_common_here" == /* ]]; then
  _common_abs="$_common_here"
elif [[ -n "${BASH_SOURCE[1]:-}" ]]; then
  _caller_dir="$(cd "$(dirname "${BASH_SOURCE[1]}")" && pwd)"
  _common_abs="$(cd "$_caller_dir/$(dirname "$_common_here")" && pwd)/$(basename "$_common_here")"
else
  _common_abs="$(cd "$(dirname "$_common_here")" && pwd)/$(basename "$_common_here")"
fi
# This file lives in tests/data/; repo root (Cargo.toml, target/) is two levels up.
_TESTS_DATA_DIR="$(cd "$(dirname "$_common_abs")" && pwd)"
GRAPH_RUN_ROOT="$(cd "$_TESTS_DATA_DIR/../.." && pwd)"

if [[ -n "${CARGO_TARGET_DIR:-}" && ! -d "$CARGO_TARGET_DIR" ]]; then
  unset CARGO_TARGET_DIR
fi

GRAPH_RUN_BIN="${GRAPH_RUN_BIN:-${CARGO_TARGET_DIR:-$GRAPH_RUN_ROOT/target}/debug/graph_run}"
if [[ ! -x "$GRAPH_RUN_BIN" ]]; then
  echo "graph_run not found or not executable at: $GRAPH_RUN_BIN" >&2
  echo "Build with: (cd \"$GRAPH_RUN_ROOT\" && cargo build --bin graph_run)" >&2
  exit 1
fi

export GRAPH_RUN_ROOT GRAPH_RUN_BIN
_GRAPH_RUN_TEST_COMMON_LOADED=1
