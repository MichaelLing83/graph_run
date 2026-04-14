# Demo: parallel “hello user” and “date/time”

Two complete **graph_run** configs (same workflow shape, different shells):

| Variant       | Directory     | Shell        | Typical host        |
|---------------|----------------|--------------|---------------------|
| Bash          | `bash/`        | `bash -l -c` | macOS, Linux, WSL   |
| PowerShell    | `powershell/` | `powershell -NoProfile -Command` | Windows |

**Quick run** (after `cargo build --bin graph_run` from the repo root):

- **Bash:** `./demos/parallel_hello_datetime/bash/run_demo.sh`
- **PowerShell:** `pwsh -File demos/parallel_hello_datetime/powershell/run_demo.ps` (or `powershell -File …` on Windows)

From the **repository root**, you can also invoke `graph_run` manually:

**Bash**

```bash
graph_run --workspace demos/parallel_hello_datetime/bash/.workspace \
  demos/parallel_hello_datetime/bash/*.toml
```

**PowerShell** (run in `cmd` or PowerShell from repo root on Windows)

```bash
graph_run --workspace demos/parallel_hello_datetime/powershell/.workspace ^
  demos/parallel_hello_datetime/powershell/00_servers.toml ^
            demos/parallel_hello_datetime/powershell/10_shells.toml ^
            demos/parallel_hello_datetime/powershell/20_commands.toml ^
            demos/parallel_hello_datetime/powershell/30_tasks.toml ^
            demos/parallel_hello_datetime/powershell/40_workflow.toml
```

On Unix shells, use the same file list as the bash example with `powershell/` paths.

## What it does

1. After `start`, **two tasks run in parallel**: `hello_user` (greeting) and `show_time` (clock).
2. **Barrier**: `join_done` runs only after **both** branches finish.
3. Then the workflow reaches `end`.

PowerShell 7 users can edit `powershell/10_shells.toml` and set `program = "pwsh"`.
