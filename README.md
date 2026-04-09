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

Run a **workflow** (a small graph of tasks) by pointing at five TOML files: servers, shells, commands, tasks, and the workflow itself. Paths may be prefixed with `@` (optional).

```bash
graph_run \
  --servers tests/data/00_servers.toml \
  --shells tests/data/01_shells.toml \
  --commands tests/data/02_commands.toml \
  --tasks tests/data/03_tasks.toml \
  tests/data/04_workflow_linear.toml
```

`--server` is accepted as an alias for `--servers`. Use **`--workspace DIR`** to create `DIR/logs/` (per-run log files) and `DIR/tmp/` (scratch space). Local tasks also receive `GRAPH_RUN_WORKSPACE` and `GRAPH_RUN_TMP` in their environment.

**Built-in control nodes:** if you omit `[[nodes]]` for **`start`**, **`end`**, or **`abort`**, they are added automatically with `type = "start"`, `"end"`, and `"abort"`. Define them explicitly when you want a custom `name` or other fields.

**Failure branch:** every `[[edges]]` row includes a **`failure`** target (where to go if the `from` task fails). If you omit it, it defaults to **`abort`**, so failed tasks end the run with a nonzero exit unless you point `failure` at another node.

**Success-edge cycles:** if the workflow’s **success** transitions (`from → to` in each `[[edges]]` row) contain a **directed cycle**, execution could run forever while every task succeeds. By default `graph_run` **refuses** such workflows and prints an error. Pass **`--allow-endless-loop`** only when that behavior is intentional (for example `tests/data/04_workflow.toml` in this repo is cyclic).

**Counted loops (`type = "loop"`):** each **success** edge from the loop node is a **body entry** (one or more targets; multiple rows mean a parallel body, like any other fan-out). A matching **`type = "loop_end"`** node with **`loop = "<loop-id>"`** ends each pass. After the last pass, execution follows the **`loop_end` node’s** success edges (not the loop node’s). Use **`count = 0`** to skip the body and jump straight to those **`loop_end`** successors. Each body task run sets **`GRAPH_RUN_LOOP_*`** env vars; **`GRAPH_RUN_LOOP_BODY_ENTRY`** / **`GRAPH_RUN_LOOP_BODY_ID`** list body entry ids (comma-separated if there are several). See **`tests/data/04_workflow_loop.toml`**.

**Logging:** use **`-v` / `--verbose`** (repeat for more detail). Without `RUST_LOG`, levels for the `graph_run` logger are: default **error**; **`-v`** → warn; **`-vv`** → info; **`-vvv`** → debug; **`-vvvv`**+ → trace. stderr uses `env_logger` timestamps. Workspace log files get the same levels (lines are prefixed with `[INFO]` etc.). If **`RUST_LOG`** is set (e.g. `RUST_LOG=graph_run=debug`), it overrides the `--verbose` mapping.

Today **local** servers run commands on this machine using the configured shell and merged environment; **remote** servers are reserved for a future SSH/telnet layer.

## Copying files and directories

`graph_run` does not implement copy itself: you run a **shell command** from **`[[commands]]`**, bound by **`[[tasks]]`** to a **server** + **shell**. The same workflow can call different tasks on different machines if you give each OS its own command + shell + task (or server) profile.

Set paths via environment (e.g. in **`[[commands.env]]`**, **`[[shells.env]]`**, or the parent process) so one workflow can reuse the same graph with different inputs. Examples below use **`GRAPH_RUN_COPY_SRC`** and **`GRAPH_RUN_COPY_DST`**.

### Server fields in the task environment (approach A: `scp` / `rsync`)

Every task inherits **`[[servers]]`** metadata from its **`server_id`** as environment variables (applied after **`[[commands]]`** / workspace env, and before loop-related extras such as **`GRAPH_RUN_LOOP_*`**):

| Variable | Meaning |
|----------|---------|
| `GRAPH_RUN_SERVER_ID` | Server row `id` |
| `GRAPH_RUN_SERVER_KIND` | e.g. `local`, `remote` |
| `GRAPH_RUN_SERVER_TRANSPORT` | e.g. `ssh`, or empty if unset |
| `GRAPH_RUN_SERVER_HOST` | Hostname or IP, or empty |
| `GRAPH_RUN_SERVER_PORT` | Port as decimal string, or empty (then your script should default SSH to 22) |
| `GRAPH_RUN_SERVER_USER` | Login user, or empty |
| `GRAPH_RUN_SERVER_DESCRIPTION` | Optional description, or empty |
| `GRAPH_RUN_SSH_USERHOST` | `user@host` when both are set; otherwise empty (handy for `scp` / `rsync` targets) |

**Passwords:** do **not** put passwords in TOML. Optionally set **`password_env`** on a server to the name of an environment variable **defined in the process that launches `graph_run`** (e.g. `export STAGING_SSH_PASS=…`). If that variable is set, its value is copied into the child as **`GRAPH_RUN_SERVER_PASSWORD`** for tools that insist on a password (discouraged vs SSH keys). If it is unset, `GRAPH_RUN_SERVER_PASSWORD` is not added.

**Cross-host copy** still runs as a **shell command on one machine** (today: always local). Typical pattern: a task on the runner (or on a bastion) uses `rsync`/`scp` with **`GRAPH_RUN_SSH_USERHOST`**, **`GRAPH_RUN_SERVER_PORT`**, and paths you supply (e.g. `GRAPH_RUN_COPY_SRC` / a remote path variable):

```toml
[[commands]]
id = "posix-rsync-to-remote-dir"
command = 'rsync -a -e "ssh -p ${GRAPH_RUN_SERVER_PORT:-22}" "$GRAPH_RUN_COPY_SRC/" "${GRAPH_RUN_SSH_USERHOST}:$GRAPH_RUN_REMOTE_DST/"'
```

Set **`GRAPH_RUN_REMOTE_DST`** (and copy sources) via **`[[commands.env]]`** or the parent environment. Adjust quoting for your shell. Prefer **SSH keys** (`ssh-agent`) over `GRAPH_RUN_SERVER_PASSWORD` / `sshpass`.

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
