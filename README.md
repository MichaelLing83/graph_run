# graph_run

`graph_run` is a small command-line program written in Rust. It is early in development; the binary currently runs and prints a short line so you can confirm the install works.

## What you need

To build from source you need a [Rust toolchain](https://www.rust-lang.org/tools/install) (`rustc` and `cargo`). You do not need Rust if someone gives you a prebuilt executable for your platform.

## Install from source

From a clone of this repository:

```bash
cargo install --path . --locked
```

That installs `graph_run` into Cargo’s bin directory (often `~/.cargo/bin`). Ensure that directory is on your `PATH`.

Alternatively, build without installing:

```bash
cargo build --release
```

The executable is `target/release/graph_run` (on Windows, `target\release\graph_run.exe`).

## Usage

Run the program by name:

```bash
graph_run
```

You should see a single line of output identifying the tool. More commands and options will be added as the project grows.

## Getting help

If something fails to build or run, open an issue in the project’s issue tracker with your operating system, Rust version (`rustc --version`), and the full error output.
