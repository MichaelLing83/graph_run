# Developing graph_run

This document is for people who change the code, run tests, or produce cross-platform builds.

## Repository layout

| Path | Role |
|------|------|
| `Cargo.toml` / `Cargo.lock` | Package metadata and locked dependencies |
| `src/lib.rs` | Load TOML configs, build workflow graph, run tasks |
| `src/main.rs` | CLI (`clap`): run with positional TOML paths, plus `visualize` subcommand (`--format mermaid\|ascii`, optional `-o`) |
| `src/workspace.rs` | Workspace layout: `logs/`, `tmp/` |
| `src/logging.rs` | `log` + `env_logger`; `--verbose` / `RUST_LOG` |
| `src/config.rs` / `workflow.rs` / `execute.rs` / `env_merge.rs` / `error.rs` | Config types, graph execution, local runner, env merge, errors |
| `tests/data/<case>/` | Per-scenario TOML fixtures; each case directory has **`run_case.sh`** to invoke `graph_run` the same way as integration tests (see script headers for options / expected exit codes). Shared **`tests/data/_common.sh`** resolves the repo root and `GRAPH_RUN_BIN`. |
| `test.sh` | Runs the test suite (`cargo test`) |
| `build.sh` | Debug + release builds; optional multi-target release builds |

## Prerequisites

- **Rust**: install via [rustup](https://rustup.rs/). Stable is enough unless you opt into nightly-only features.
- **Cross-compilation (optional)**:
  - **cross** (`cargo install cross`) with **Docker Desktop** (or another engine) running, or
  - **Zig** plus **cargo-zigbuild** (`cargo install cargo-zigbuild`) if you prefer not to use Docker.

## Everyday workflow

```bash
./test.sh              # same as cargo test; extra args are forwarded
./build.sh             # cargo build + cargo build --release (host only)
cargo run -- \
  tests/data/workflow_linear/00_servers.toml tests/data/workflow_linear/01_shells.toml \
  tests/data/workflow_linear/02_commands.toml tests/data/workflow_linear/03_tasks.toml \
  tests/data/workflow_linear/04_workflow_linear.toml \
  --workspace target/graph_run_workspace
cargo run -- visualize --format mermaid \
  tests/data/workflow_fork_join/00_servers.toml tests/data/workflow_fork_join/01_shells.toml \
  tests/data/workflow_fork_join/02_commands.toml tests/data/workflow_fork_join/03_tasks.toml \
  tests/data/workflow_fork_join/04_workflow_fork_join.toml
```

Run Clippy or formatting the way you usually do for Rust projects (`cargo clippy`, `cargo fmt`).

## Cross-building release binaries

```bash
./build.sh --all-targets
```

This always performs a **native** debug and release build, then release builds for a fixed set of targets (see `TARGETS` in `build.sh`).

**How linking is chosen**

1. Targets on the **same OS** as the host (e.g. `x86_64-apple-darwin` on Apple Silicon) use plain `cargo`.
2. Otherwise the script tries **`cross`** only if `docker version` succeeds (so a stale `cross` install does not blindly fall back to a broken host link).
3. If `cross` is not used, it tries **`cargo zigbuild`** when available.
4. If neither can handle the target, the script exits with a short message about installing Docker + `cross` or Zig + `cargo-zigbuild`.

**Targets included today**

- macOS: `x86_64-apple-darwin`, `aarch64-apple-darwin` (the one matching the host is skipped as redundant with the native release step)
- Linux: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`
- Windows: `x86_64-pc-windows-gnu` (MinGW in the cross image)

**Windows on ARM (MSVC)** is not in the script: `cross` does not provide a default image for `aarch64-pc-windows-msvc`, and host-side fallback would require MSVC `link.exe` on Windows. Build that triple on a Windows machine or in Windows CI with the Visual Studio build tools.

Environment knobs (all optional):

- `USE_CROSS=0` — do not use `cross` even if installed and Docker works.
- `USE_ZIGBUILD=0` — do not use `cargo zigbuild`.

Release artifacts land under `target/<triple>/release/` with the binary named `graph_run` (and `.exe` on Windows).

## Tests

`./test.sh` forwards arguments to `cargo test`, for example:

```bash
./test.sh -- --nocapture
```

## Versioning and releases

The crate version lives in `Cargo.toml`. Tagging and publishing to [crates.io](https://crates.io/) are optional; if you add a crate name reservation or CI, document the exact steps here.

## License

See `Cargo.toml` (`MIT OR Apache-2.0` as declared there). If you add a `LICENSE` file, keep it in sync with that choice.
