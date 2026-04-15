#!/usr/bin/env bash
# Build graph_run: native debug + release, copy release to ~/.cargo/bin (or $CARGO_HOME/bin),
# then optional cross-target release binaries.
#
# Native (always):
#   ./build.sh
#
# Also build release for common Linux / macOS / Windows triples:
#   ./build.sh --all-targets
#
# Cross-compiling to Linux/Windows from macOS (or other OS mismatches) needs a linker.
# This script picks, in order:
#   1. cross (cargo install cross; Docker must be running) — unless USE_CROSS=0
#   2. cargo zigbuild (cargo install cargo-zigbuild; install Zig) — unless USE_ZIGBUILD=0
# Same-OS targets (e.g. x86_64-apple-darwin on Apple Silicon) use plain cargo. OpenSSL is built
# from source for the target (Cargo: openssl-sys `vendored`) so pkg-config is not required for
# cross-macOS builds. First build needs network to fetch OpenSSL; Perl is required by the OpenSSL build.
#
# With --all-targets, after every release build succeeds, binaries are copied into a fresh
# temporary directory as graph_run-<target-triple> (and .exe for Windows GNU).
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

target_os() {
  case "$1" in
    *-apple-darwin) echo apple ;;
    *-linux-*) echo linux ;;
    *-windows-*) echo windows ;;
    *) echo unknown ;;
  esac
}

HOST_OS="$(target_os "$HOST")"

same_os_as_host() {
  [[ "$(target_os "$1")" == "$HOST_OS" ]]
}

docker_ok() {
  docker version >/dev/null 2>&1
}

have_cross() {
  [[ "${USE_CROSS:-}" != "0" ]] &&
    command -v cross >/dev/null 2>&1 &&
    docker_ok
}

have_zigbuild() {
  [[ "${USE_ZIGBUILD:-}" != "0" ]] && cargo zigbuild --version >/dev/null 2>&1
}

build_release_target() {
  local t=$1
  if same_os_as_host "$t"; then
    cargo build --release --target "$t"
    return
  fi
  if have_cross; then
    cross build --release --target "$t"
    return
  fi
  if have_zigbuild; then
    cargo zigbuild --release --target "$t"
    return
  fi
  echo "Cannot link $t from host $HOST (Apple's linker cannot produce Linux/Windows binaries)." >&2
  echo "Install one of:" >&2
  echo "  - Docker + cargo install cross   (then re-run ./build.sh --all-targets)" >&2
  echo "  - Zig + cargo install cargo-zigbuild" >&2
  exit 1
}

echo "Host triple: $HOST"
echo "==> Debug (native)"
cargo build

echo "==> Release (native)"
cargo build --release

CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin"
mkdir -p "$CARGO_BIN"
NATIVE_RELEASE="$ROOT/target/release/graph_run"
if [[ -f "$NATIVE_RELEASE" ]]; then
  cp -f "$NATIVE_RELEASE" "$CARGO_BIN/graph_run"
  echo "Installed: $CARGO_BIN/graph_run"
elif [[ -f "$NATIVE_RELEASE.exe" ]]; then
  cp -f "$NATIVE_RELEASE.exe" "$CARGO_BIN/graph_run.exe"
  echo "Installed: $CARGO_BIN/graph_run.exe"
else
  echo "warning: native release binary not found at $NATIVE_RELEASE (skipped install)" >&2
fi

if [[ "$ALL_TARGETS" -eq 0 ]]; then
  echo "Done (native only). Pass --all-targets for Linux/macOS/Windows release builds."
  exit 0
fi

# Same-OS targets first; then Linux and Windows GNU. Windows ARM64 (aarch64-pc-windows-msvc) is
# omitted: cross-rs has no image for it, and host fallback needs MSVC link.exe (Windows-only).
TARGETS=(
  x86_64-apple-darwin
  aarch64-apple-darwin
  x86_64-unknown-linux-gnu
  aarch64-unknown-linux-gnu
  x86_64-pc-windows-gnu
)

for t in "${TARGETS[@]}"; do
  if [[ "$t" == "$HOST" ]]; then
    echo "==> Skip release --target $t (same as host)"
    continue
  fi
  echo "==> Release for $t"
  rustup target add "$t" 2>/dev/null || true
  build_release_target "$t"
done

# One directory with every cross-built (and host) release binary, named by Rust target triple.
BUNDLE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/graph_run_all_targets.XXXXXX")"
for t in "${TARGETS[@]}"; do
  if [[ "$t" == "$HOST" ]]; then
    src="$ROOT/target/release/graph_run"
    if [[ ! -f "$src" ]]; then
      src="$ROOT/target/release/graph_run.exe"
    fi
  else
    src="$ROOT/target/$t/release/graph_run"
    if [[ ! -f "$src" ]]; then
      src="$ROOT/target/$t/release/graph_run.exe"
    fi
  fi
  if [[ ! -f "$src" ]]; then
    echo "warning: expected release binary missing for $t (skip copy): $src" >&2
    continue
  fi
  if [[ "$src" == *.exe ]]; then
    cp -f "$src" "$BUNDLE_DIR/graph_run-${t}.exe"
  else
    cp -f "$src" "$BUNDLE_DIR/graph_run-${t}"
  fi
done

echo "Per-target release binaries collected in: $BUNDLE_DIR"
echo "(Also under target/<triple>/release/ as usual.)"
