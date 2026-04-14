# Demo: parallel “hello user” and “date/time”

Two complete **graph_run** configs (same workflow shape, different shells):

| Variant       | Directory     | Shell        | Typical host        |
|---------------|----------------|--------------|---------------------|
| Bash          | `bash/`        | `bash -l -c` | macOS, Linux, WSL   |
| PowerShell    | `powershell/` | `powershell -NoProfile -Command` | Windows |

From the **repository root**, run one variant (flags before `--configs`):

**Bash**

```bash
graph_run --workspace demos/parallel_hello_datetime/bash/.workspace \
  --configs demos/parallel_hello_datetime/bash/*.toml
```

**PowerShell** (run in `cmd` or PowerShell from repo root on Windows)

```bash
graph_run --workspace demos/parallel_hello_datetime/powershell/.workspace ^
  --configs demos/parallel_hello_datetime/powershell/00_servers.toml ^
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
