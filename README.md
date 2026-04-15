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

**Transfer tasks (`transfer` in `[[tasks]]`):** instead of `server_id` / `shell_id` / `command_id`, set **`transfer`** to an inline table or add a **`[tasks.transfer]`** section immediately under that `[[tasks]]` row. Copies run over **SFTP** (`libssh2`) between two **`[[servers]]`** rows. Each side may be **`kind = "local"`** (paths on the host running `graph_run`) or **`kind = "remote"`** (`host`, `user`, `port`, plus **`password`** / **`password_env`** or SSH agent). Mode and mtime are applied on the destination where SFTP allows (similar intent to **`rsync -a`**; only regular files, directories, and symlinks are supported). A **trailing slash** on **`source_path`** means “copy directory *contents* into **`dest_path`**”; without it, the directory tree is created under **`dest_path`**. Timeout is the **minimum** of the task **`timeout`** and both servers’ **`timeout`**, or **300s** if none are set. Building `graph_run` compiles **OpenSSL** and **libssh2** for your target (see **What you need** above); no separate `brew install openssl` for linking.

In **`source_path`** / **`dest_path`**, SFTP does not run a shell: **`$HOME`**, **`$GRAPH_RUN_WORKSPACE`**, and **`$GRAPH_RUN_TMP`** are expanded by `graph_run` before opening files. (Some SFTP servers return **absolute** paths from directory listings; `graph_run` maps those under your destination so local paths are not accidentally rooted at `/`.) **`$GRAPH_RUN_*`** use the configured workspace directory (CLI **`--workspace`** or default **`.workspace`**). **`$HOME`** on a **local** path uses the graph_run process environment; on a **remote** path it is resolved with a short **`sh`** **`exec`** on that SSH session (same login account as SFTP), then SFTP uses the resulting path.

```toml
[[tasks]]
id = "sync-artifacts"
transfer = { source_server_id = "prod", dest_server_id = "local", source_path = "/var/out/", dest_path = "artifacts" }
```

**Built-in control nodes:** if you omit `[[nodes]]` for **`start`**, **`end`**, or **`abort`**, they are added automatically with `type = "start"`, `"end"`, and `"abort"`. Define them explicitly when you want a custom `name` or other fields.

**Implicit task nodes:** for every **`[[tasks]]`** row, a matching workflow node with the same **`id`** and default **`type = "task"`** is added when no `[[nodes]]` row with that **`id`** exists. If a node **`id`** already exists but is **not** a task (for example a **`loop`**), config loading fails with a clear error—you cannot reuse the same id as both a task and another node kind.

**Failure branch:** every `[[edges]]` row includes a **`failure`** target (where to go if the `from` task fails). If you omit it, it defaults to **`abort`**, so failed tasks end the run with a nonzero exit unless you point `failure` at another node.

**Retries (`retry` on `[[tasks]]`):** optional non-negative integer (default **`0`**). After a **failed** attempt—a command exits non-zero, or a **transfer** returns an error (SFTP/SSH/local copy failure, missing path, etc.)—`graph_run` may run the **same** task again up to **`retry`** additional times. If attempts are exhausted, the workflow follows the task’s **`failure`** edge (same as a failed command), after logging the last error. Invalid configs are still rejected when configs are **loaded**, before the workflow runs. The total number of attempts is **`1 + retry`**. Values above **10000** are rejected at load time.

**Success-edge cycles:** if the workflow’s **success** transitions (`from → to` in each `[[edges]]` row) contain a **directed cycle**, execution could run forever while every task succeeds. By default `graph_run` **refuses** such workflows and prints an error. Pass **`--allow-endless-loop`** only when that behavior is intentional (for example `tests/data/workflow_cyclic/04_workflow.toml` in this repo is cyclic).

**Counted loops (`type = "loop"`):** each **success** edge from the loop node is a **body entry** (one or more targets; multiple rows mean a parallel body, like any other fan-out). A matching **`type = "loop_end"`** node with **`loop = "<loop-id>"`** ends each pass. After the last pass, execution follows the **`loop_end` node’s** success edges (not the loop node’s). Use **`count = 0`** to skip the body and jump straight to those **`loop_end`** successors. Each body task run sets **`GRAPH_RUN_LOOP_*`** env vars; **`GRAPH_RUN_LOOP_BODY_ENTRY`** / **`GRAPH_RUN_LOOP_BODY_ID`** list body entry ids (comma-separated if there are several). See **`tests/data/workflow_loop/04_workflow_loop.toml`**.

**Logging:** use **`-v` / `--verbose`** (repeat for more detail). Without `RUST_LOG`, levels for the `graph_run` logger are: default **error**; **`-v`** → warn; **`-vv`** → info; **`-vvv`** → debug; **`-vvvv`**+ → trace. stderr uses `env_logger` timestamps. Workspace log files get the same levels (lines are prefixed with `[INFO]` etc.). If **`RUST_LOG`** is set (e.g. `RUST_LOG=graph_run=debug`), it overrides the `--verbose` mapping.

**Local** servers run commands on this machine using the configured shell and merged environment (including the graph_run process environment). **Remote** servers (`kind = "remote"`) run the same shell + command line over **SSH** (`exec`, same auth as transfer tasks: password, then SSH agent). Remote tasks **do not** inherit the graph_run host’s process environment, so literals like **`$HOME`** in **`[[commands]]`** `command` / `cwd` expand only on the remote machine after login-shell setup. **`[[shells.env]]` / `[[commands.env]]` / `[[tasks.env]]`** and **`GRAPH_RUN_*`** are still applied. **Remote command execution is Unix-only** (non-Unix hosts get a clear error for `kind = "remote"` command tasks). **Transfer** tasks use SFTP between two server rows as described above.

## Copying files and directories

Besides **built-in transfer tasks** (SFTP between two **`[[servers]]`** rows, described above), you can copy by running a **shell command** from **`[[commands]]`**, bound by **`[[tasks]]`** to a **server** + **shell**. The same workflow can call different tasks on different machines if you give each OS its own command + shell + task (or server) profile.

Set paths via environment (e.g. in **`[[commands.env]]`**, **`[[shells.env]]`**, or the parent process) so one workflow can reuse the same graph with different inputs. Examples below use **`GRAPH_RUN_COPY_SRC`** and **`GRAPH_RUN_COPY_DST`**.

### Server fields in the task environment (approach A: `scp` / `rsync` from a local shell)

**Which `server_id`?** For **command** tasks, **`server_id`** selects the **`[[servers]]`** row whose **`kind`** is **`local`** (run on the host that executes `graph_run`) or **`remote`** (run over SSH on Unix, as in the previous section). For “push files from my laptop to a remote host with **`rsync`**” **without** using a transfer task, it is common to keep **`server_id = "local"`** so the shell runs on the laptop; you pass the remote SSH destination in **`[[commands.env]]`** or the parent environment (see **`GRAPH_RUN_REMOTE_*`** below).

**What gets injected:** every task inherits **`[[servers]]`** fields from **that same** **`server_id`** as `GRAPH_RUN_SERVER_*` (merged **after** per-command env from **`[[commands.env]]`**, so server keys **override** duplicate names from the command). For a typical **`local`** row, `host` / `user` / `port` are unset, so **`GRAPH_RUN_SSH_USERHOST`** and related SSH fields are **empty**. For a **`remote`** row, those fields describe the SSH endpoint used for that task.

For **local → remote** copy driven by **`rsync`** from a **local** task, pass destination host, port, and paths yourself—usually **`[[commands.env]]`** or the parent process—using names that **do not** collide with `GRAPH_RUN_SERVER_*` when you need values different from the task’s server row (for example the `GRAPH_RUN_REMOTE_*` names below).

| Variable | Meaning |
|----------|---------|
| `GRAPH_RUN_SERVER_ID` | Server row `id` |
| `GRAPH_RUN_SERVER_KIND` | e.g. `local`, `remote` |
| `GRAPH_RUN_SERVER_TRANSPORT` | e.g. `ssh`, or empty if unset |
| `GRAPH_RUN_SERVER_HOST` | Hostname or IP for **this** server row, or empty |
| `GRAPH_RUN_SERVER_PORT` | Port as decimal string, or empty (default SSH in scripts is often 22) |
| `GRAPH_RUN_SERVER_USER` | Login user for **this** row, or empty |
| `GRAPH_RUN_SERVER_DESCRIPTION` | Optional description, or empty |
| `GRAPH_RUN_SSH_USERHOST` | `user@host` when **this row’s** `user` and `host` are both set; otherwise empty (useful when the task’s server row really is the SSH endpoint, e.g. future remote-side tasks) |

**Passwords:** optional **`password`** on a server row is read from TOML and passed to the child as **`GRAPH_RUN_SERVER_PASSWORD`** (empty string is ignored). Prefer SSH keys; avoid committing real secrets—use **`--constants`** substitution or **`password_env`** instead. If **`password_env`** is set to the name of a variable **in the `graph_run` process**, that variable’s value is used when it is **defined** (even if empty), overriding **`password`**; if the name is not set in the environment, **`password`** from TOML is used. If neither yields a value, `GRAPH_RUN_SERVER_PASSWORD` is not set.

**Cross-host copy** is still one shell command on the runner. Supply the **remote** SSH user@host, port, and destination path with your own variables (here `GRAPH_RUN_REMOTE_SSH_*` and `GRAPH_RUN_REMOTE_DST`):

```toml
[[commands]]
id = "posix-rsync-to-remote-dir"
command = 'rsync -a -e "ssh -p ${GRAPH_RUN_REMOTE_SSH_PORT:-22}" "$GRAPH_RUN_COPY_SRC/" "${GRAPH_RUN_REMOTE_SSH_USERHOST}:$GRAPH_RUN_REMOTE_DST/"'

[[commands.env]]
name = "GRAPH_RUN_REMOTE_SSH_USERHOST"
strategy = "override"
value = "deploy@staging.example.com"

[[commands.env]]
name = "GRAPH_RUN_REMOTE_SSH_PORT"
strategy = "override"
value = "22"
```

Set **`GRAPH_RUN_REMOTE_DST`**, **`GRAPH_RUN_COPY_SRC`**, and real credentials via more **`[[commands.env]]`** rows or the parent environment. Adjust quoting for your shell. Prefer **SSH keys** (`ssh-agent`) over `GRAPH_RUN_SERVER_PASSWORD` / `sshpass`.

### Linux and macOS (bash / zsh / POSIX `sh`)

Use **`cp`** for a single file or a **recursive** directory tree. **`cp -a`** preserves metadata where the platform allows (timestamps, permissions; follows platform `cp` behavior).

**Single file**

```toml
[[commands]]
id = "posix-copy-file"
command = 'cp -f -- "$GRAPH_RUN_COPY_SRC" "$GRAPH_RUN_COPY_DST"'
```

**Directory (recursive)**

```toml
[[commands]]
id = "posix-copy-dir-recursive"
command = 'cp -a -- "$GRAPH_RUN_COPY_SRC" "$GRAPH_RUN_COPY_DST"'
```

Create the destination parent directory first if needed, e.g. add a preceding task with `mkdir -p -- "$(dirname "$GRAPH_RUN_COPY_DST")"` (file) or ensure `GRAPH_RUN_COPY_DST`’s parent exists (directory copy).

**Optional: `rsync`** (often installed on Linux/macOS; good for “mirror” semantics). Requires `rsync` on the target host.

```toml
[[commands]]
id = "posix-rsync-dir-recursive"
command = 'rsync -a --delete -- "$GRAPH_RUN_COPY_SRC/" "$GRAPH_RUN_COPY_DST/"'
```

Adjust flags (`--delete` is destructive); omit it for a conservative first copy.

### Windows (PowerShell)

Use a **`[[shells]]`** entry whose **`program`** is **`pwsh`** or **`powershell`**, and match it in **`[[tasks]]`**. **`Copy-Item`** returns a straightforward exit code for automation.

**Single file**

```toml
[[commands]]
id = "pwsh-copy-file"
command = 'Copy-Item -LiteralPath $env:GRAPH_RUN_COPY_SRC -Destination $env:GRAPH_RUN_COPY_DST -Force'
```

**Directory (recursive)**

```toml
[[commands]]
id = "pwsh-copy-dir-recursive"
command = 'Copy-Item -LiteralPath $env:GRAPH_RUN_COPY_SRC -Destination $env:GRAPH_RUN_COPY_DST -Recurse -Force'
```

### Windows (`cmd.exe` and `robocopy`)

**`cmd` `copy`** is fine for a **single file** (`copy /Y`). For **whole directories**, **`robocopy`** is common on servers but its **exit codes are not a simple 0 = success** (values 0–7 can indicate success with different meanings). Prefer **PowerShell `Copy-Item`** above unless you already wrap `robocopy` and normalize exit status.

### Wiring tasks and shells

- Give each **OS + shell** combination a dedicated **`[[commands]]`** row (or duplicate ids per server file if you split inventory by environment).
- In **`[[tasks]]`**, set **`server_id`**, **`shell_id`**, and **`command_id`** so a Linux host runs `posix-copy-*` under bash, and a Windows host runs `pwsh-copy-*` under PowerShell.
- If a command string uses **`$VAR`** (POSIX) vs **`$env:VAR`** (PowerShell), the **wrong shell** will fail or mis-parse: keep command strings and **`shell_id`** aligned.

## Getting help

If something fails to build or run, open an issue in the project’s issue tracker with your operating system, Rust version (`rustc --version`), and the full error output.
