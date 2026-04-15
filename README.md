# graph_run

`graph_run` is a Rust CLI that loads merged TOML configs, builds a workflow graph (`[[nodes]]` / `[[edges]]`), and runs **command** or **transfer** tasks on **local** or **remote** servers. It can **visualize** the graph and **merge** configs into one normalized file.

## What you need

To build from source you need a [Rust toolchain](https://www.rust-lang.org/tools/install) (`rustc` and `cargo`). **OpenSSL** for SSH/SFTP is compiled **from source** for your target via the `openssl-sys` **`vendored`** feature (no system `libssl` required). The OpenSSL build needs **Perl** in `PATH`. **libssh2** is still built by `libssh2-sys` (often via its own bundled sources). Cross-compiling to another macOS architecture (e.g. x86_64 from Apple Silicon) uses that same vendored OpenSSL for the target. You do not need Rust if someone gives you a prebuilt executable for your platform.

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

Run a **workflow** by passing one or more TOML paths as **positional arguments** (at least one **`FILE`**). Each file may define any subset of **`[[servers]]`**, **`[[shells]]`**, **`[[commands]]`**, **`[[tasks]]`**, **`[[nodes]]`**, and **`[[edges]]`**. Multiple files are **merged in order**: within each section, rows from earlier files come before rows from later files. The effective order of processing is always servers → shells → commands → tasks → workflow (`nodes` / `edges`). You can use a **single** file that contains every section, or **split** them across several paths (for example under `tests/data/workflow_linear/` in this repo). Paths may be prefixed with `@` (optional).

```bash
graph_run \
  tests/data/workflow_linear/00_servers.toml \
  tests/data/workflow_linear/01_shells.toml \
  tests/data/workflow_linear/02_commands.toml \
  tests/data/workflow_linear/03_tasks.toml \
  tests/data/workflow_linear/04_workflow_linear.toml
```

Options such as **`-v`** and **`--workspace`** can appear before or after the file list. If a path starts with **`-`**, put **`--`** before it so it is not parsed as a flag.

To visualize merged workflow configs without running tasks, use the **`visualize`** subcommand:

```bash
graph_run visualize \
  --format mermaid \
  tests/data/workflow_fork_join/00_servers.toml \
  tests/data/workflow_fork_join/01_shells.toml \
  tests/data/workflow_fork_join/02_commands.toml \
  tests/data/workflow_fork_join/03_tasks.toml \
  tests/data/workflow_fork_join/04_workflow_fork_join.toml
```

Visualization formats are **`mermaid`** (default) and **`ascii`**. Use **`-o FILE`** to write output to a file instead of stdout. This command supports **`--constants FILE`** and the same positional config paths as normal runs.

To merge multiple config files into a single normalized TOML document, use **`merge`**:

```bash
graph_run merge \
  --output merged.toml \
  tests/data/workflow_linear/00_servers.toml \
  tests/data/workflow_linear/01_shells.toml \
  tests/data/workflow_linear/02_commands.toml \
  tests/data/workflow_linear/03_tasks.toml \
  tests/data/workflow_linear/04_workflow_linear.toml
```

`merge` preserves behavior while normalizing ordering/format. **`[[nodes]]` rows that were only implied by the loader are omitted from output:** default control nodes (`start`, `end`, `abort`) unless they appeared in an input file, and default **task** nodes for each `[[tasks]].id` unless that id had a `[[nodes]]` row in the merged input (so you can author workflows with only `[[tasks]]` + `[[edges]]` when a plain task node is enough). The command also supports `--constants FILE`.

**`--workspace DIR`** sets where `graph_run` creates `DIR/logs/` (per-run log files) and `DIR/tmp/` (scratch space); local tasks receive `GRAPH_RUN_WORKSPACE` and `GRAPH_RUN_TMP`. If you omit **`--workspace`**, the default is **`.workspace`** in the current working directory (override with **`--workspace /path/to/dir`**).

**Constants (`--constants FILE`):** optional substitution pass for sharing repeated values across your config TOML files (plain TOML has no variables). The constants file is a single table of scalars, for example:

```toml
STAGING_HOST = "10.0.0.5"
DEPLOY_PORT = 22
```

In each config file, write **`${STAGING_HOST}`** (name must match `[A-Za-z0-9_]+`). Each occurrence is replaced with the string form of that value **before** TOML parsing. The constants file itself is not expanded. Unknown `${NAME}` or an unclosed `${` is an error. Omit **`--constants`** to leave configs unchanged.

### File transfer (transfer tasks)

Use a **transfer task** to copy files or directories between two **`[[servers]]`** over **SFTP** (`libssh2`)—no **`[[commands]]`** or shell on the paths involved. Supported pairs are **local→local**, **local→remote**, **remote→local**, and **remote→remote** (each side is a **`[[servers]]`** row with **`kind = "local"`** or **`kind = "remote"`**).

1. **Define two servers** in **`[[servers]]`**. Each may be **`kind = "local"`** (paths on the machine running `graph_run`) or **`kind = "remote"`** (typically **`transport = "ssh"`**, **`host`**, **`user`**, optional **`port`**). Use stable **`id`** values; those ids are referenced from the task.

2. **Add a `[[tasks]]` row** that sets **`transfer`** to an inline table, or add **`[tasks.transfer]`** directly under that row. Fields:

   | Field | Meaning |
   |-------|---------|
   | **`source_server_id`** | **`[[servers]].id`** for the side you read from |
   | **`dest_server_id`** | **`[[servers]].id`** for the side you write to |
   | **`source_path`** | Path on the source server (POSIX **`/`** on remote) |
   | **`dest_path`** | Path on the destination server |

   On the same row, **do not** set **`server_id`**, **`shell_id`**, or **`command_id`** (those are for command tasks only).

3. **Schedule it** like any other task: add **`[[edges]]`** to/from the task’s **`id`**. You can omit a **`[[nodes]]`** row for that **`id`** when a default task node is enough (see **Implicit task nodes** below).

**Path expansion:** SFTP does not run a shell on the path string. **`$HOME`**, **`$GRAPH_RUN_WORKSPACE`**, and **`$GRAPH_RUN_TMP`** in **`source_path`** / **`dest_path`** are expanded by `graph_run` before opening files. **`$GRAPH_RUN_*`** use **`--workspace`** (default **`.workspace`**). **`$HOME`** on a **local** path uses the graph_run process environment; on a **remote** path, HOME is resolved once on that host via a short **`sh`** **`exec`** (same SSH account as SFTP). Some SFTP servers return **absolute** paths when listing directories; `graph_run` maps those under your destination so paths are not accidentally rooted at **`/`**.

**Trailing slash on `source_path`:** if **`source_path`** ends with **`/`**, the **contents** of that directory are copied into **`dest_path`**. Without a trailing slash, the source directory itself is created under **`dest_path`**.

**What gets copied:** regular files, directories, and symlinks only. Mode and mtime are applied on the destination where SFTP allows. The effective timeout is the **minimum** of the task **`timeout`** and both servers’ **`timeout`**, or **300s** if none are set.

**Authentication (remote):** use SSH keys when possible. Optional **`password`** on the server row is used for SFTP/SSH; **`password_env`** names a variable in the **graph_run** process whose value overrides **`password`** when that variable is set (even to empty). Building `graph_run` vendors **OpenSSL** and **libssh2** (see **What you need**).

Example (remote tree → local workspace scratch):

```toml
[[servers]]
id = "local"
kind = "local"

[[servers]]
id = "backup"
kind = "remote"
transport = "ssh"
host = "files.example.com"
user = "deploy"

[[tasks]]
id = "pull-backup"
transfer = { source_server_id = "backup", dest_server_id = "local", source_path = "/var/backups/", dest_path = "$GRAPH_RUN_TMP/restore/" }
```

A fuller remote→local fixture lives under **`tests/data/test_file_transfer/`** in this repository.

**Built-in control nodes:** if you omit `[[nodes]]` for **`start`**, **`end`**, or **`abort`**, they are added automatically with `type = "start"`, `"end"`, and `"abort"`. Define them explicitly when you want a custom `name` or other fields.

**Implicit task nodes:** for every **`[[tasks]]`** row, a matching workflow node with the same **`id`** and default **`type = "task"`** is added when no `[[nodes]]` row with that **`id`** exists. If a node **`id`** already exists but is **not** a task (for example a **`loop`**), config loading fails with a clear error—you cannot reuse the same id as both a task and another node kind.

**Failure branch:** every `[[edges]]` row includes a **`failure`** target (where to go if the `from` task fails). If you omit it, it defaults to **`abort`**, so failed tasks end the run with a nonzero exit unless you point `failure` at another node.

**Retries (`retry` on `[[tasks]]`):** optional non-negative integer (default **`0`**). After a **failed** attempt—a command exits non-zero, or a **transfer** returns an error (SFTP/SSH/local copy failure, missing path, etc.)—`graph_run` may run the **same** task again up to **`retry`** additional times. If attempts are exhausted, the workflow follows the task’s **`failure`** edge (same as a failed command), after logging the last error. Invalid configs are still rejected when configs are **loaded**, before the workflow runs. The total number of attempts is **`1 + retry`**. Values above **10000** are rejected at load time.

**Success-edge cycles:** if the workflow’s **success** transitions (`from → to` in each `[[edges]]` row) contain a **directed cycle**, execution could run forever while every task succeeds. By default `graph_run` **refuses** such workflows and prints an error. Pass **`--allow-endless-loop`** only when that behavior is intentional (for example `tests/data/workflow_cyclic/04_workflow.toml` in this repo is cyclic).

**Counted loops (`type = "loop"`):** each **success** edge from the loop node is a **body entry** (one or more targets; multiple rows mean a parallel body, like any other fan-out). A matching **`type = "loop_end"`** node with **`loop = "<loop-id>"`** ends each pass. After the last pass, execution follows the **`loop_end` node’s** success edges (not the loop node’s). Use **`count = 0`** to skip the body and jump straight to those **`loop_end`** successors. Each body task run sets **`GRAPH_RUN_LOOP_*`** env vars; **`GRAPH_RUN_LOOP_BODY_ENTRY`** / **`GRAPH_RUN_LOOP_BODY_ID`** list body entry ids (comma-separated if there are several). See **`tests/data/workflow_loop/04_workflow_loop.toml`**.

**Logging:** use **`-v` / `--verbose`** (repeat for more detail). Without `RUST_LOG`, levels for the `graph_run` logger are: default **error**; **`-v`** → warn; **`-vv`** → info; **`-vvv`** → debug; **`-vvvv`**+ → trace. stderr uses `env_logger` timestamps. Workspace log files get the same levels (lines are prefixed with `[INFO]` etc.). If **`RUST_LOG`** is set (e.g. `RUST_LOG=graph_run=debug`), it overrides the `--verbose` mapping.

**Local** servers run commands on this machine using the configured shell and merged environment (including the graph_run process environment). **Remote** servers (`kind = "remote"`) run the same shell + command line over **SSH** (`exec`, same auth model as transfer tasks: password, then SSH agent). Remote command tasks **do not** inherit the graph_run host’s process environment, so literals like **`$HOME`** in **`[[commands]]`** `command` / `cwd` expand only on the remote machine after login-shell setup. **`[[shells.env]]` / `[[commands.env]]` / `[[tasks.env]]`** and **`GRAPH_RUN_*`** are still applied. **Remote command execution is Unix-only** (non-Unix hosts get a clear error for `kind = "remote"` command tasks).

Every **command** task inherits **`GRAPH_RUN_SERVER_*`** and related fields from its **`server_id`** row (merged after command env). For a **`local`** row, **`GRAPH_RUN_SSH_USERHOST`** is usually empty; for **`remote`**, it is set when **`user`** and **`host`** are both present. Optional **`password`** / **`password_env`** on the server row apply to SSH for command tasks the same way as for SFTP on transfer tasks.

## Getting help

If something fails to build or run, open an issue in the project’s issue tracker with your operating system, Rust version (`rustc --version`), and the full error output.
