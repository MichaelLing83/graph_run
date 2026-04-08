#!/usr/bin/env bash
# Build graph_run: native debug + release, then optional cross-target release binaries.
#
# Native (always):
#   ./build.sh
#
# Also build release for common Linux / macOS / Windows triples:
#   ./build.sh --all-targets
#
# Cross-compilation usually needs either:
#   - cargo install cross && Docker running, then: USE_CROSS=1 ./build.sh --all-targets
#   - or a working linker/toolchain per target (see https://doc.rust-lang.org/rustc/platform-support.html)
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

HOST="$(rustc -vV | sed -n 's/^host: //p')"

usage() {
  echo "Usage: $0 [--all-targets]" >&2
  exit 1
}

ALL_TARGETS=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --all-targets) ALL_TARGETS=1; shift ;;
    -h|--help) usage ;;
    *) usage ;;
  esac
done

run_build() {
  local mode=$1
  shift
  if [[ -n "${USE_CROSS:-}" && "${USE_CROSS}" != "0" ]]; then
    cross "$mode" "$@"
  else
    cargo "$mode" "$@"
  fi
}

echo "Host triple: $HOST"
echo "==> Debug (native)"
cargo build

echo "==> Release (native)"
cargo build --release

if [[ "$ALL_TARGETS" -eq 0 ]]; then
  echo "Done (native only). Pass --all-targets for Linux/macOS/Windows release builds."
  exit 0
fi

# Tier-1 style triples we want artifacts for.
TARGETS=(
  x86_64-unknown-linux-gnu
  aarch64-unknown-linux-gnu
  x86_64-pc-windows-gnu
  aarch64-pc-windows-gnu
  x86_64-apple-darwin
  aarch64-apple-darwin
)

use_cross_for_target() {
  local t=$1
  if [[ -z "${USE_CROSS:-}" || "${USE_CROSS}" == "0" ]]; then
    return 1
  fi
  # cross helps most when host OS != target OS
  case "$HOST" in
    *-apple-darwin)
      [[ "$t" == *-linux-* || "$t" == *-windows-* ]]
      ;;
    *-unknown-linux-gnu)
      [[ "$t" == *-apple-darwin || "$t" == *-windows-* ]]
      ;;
    *-pc-windows-msvc|*-pc-windows-gnu)
      [[ "$t" == *-linux-* || "$t" == *-apple-darwin || ( "$t" == *-windows-* && "$t" != "$HOST" ) ]]
      ;;
    *) return 0 ;;
  esac
}

for t in "${TARGETS[@]}"; do
  if [[ "$t" == "$HOST" ]]; then
    echo "==> Skip release --target $t (same as host)"
    continue
  fi
  echo "==> Release for $t"
  rustup target add "$t" 2>/dev/null || true
  if use_cross_for_target "$t"; then
    USE_CROSS=1 run_build build --release --target "$t"
  else
    cargo build --release --target "$t" || {
      echo "Failed: cargo build --release --target $t" >&2
      echo "Tip: install Docker and run: cargo install cross && USE_CROSS=1 $0 --all-targets" >&2
      exit 1
    }
  fi
done

echo "Artifacts under target/*/release/graph_run (and .exe on Windows)."
