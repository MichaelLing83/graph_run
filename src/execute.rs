use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use wait_timeout::ChildExt;

use crate::config::{Command as CmdDef, Server, Shell, Task};
use crate::env_merge::merge_entries;
use crate::error::{GraphRunError, Result};

pub fn run_task(
    server: &Server,
    shell: &Shell,
    cmd: &CmdDef,
    task: &Task,
    workspace_root: Option<&Path>,
    extra_env: &[(String, String)],
) -> Result<std::process::ExitStatus> {
    match server.kind.as_str() {
        "local" => run_local(
            server,
            shell,
            cmd,
            task,
            workspace_root,
            extra_env,
        ),
        "remote" => {
            #[cfg(unix)]
            {
                run_remote(
                    server,
                    shell,
                    cmd,
                    task,
                    workspace_root,
                    extra_env,
                )
            }
            #[cfg(not(unix))]
            {
                Err(GraphRunError::msg(format!(
                    "remote server kind {:?}: command execution over SSH is only supported on Unix (server {})",
                    server.kind, server.id
                )))
            }
        }
        other => Err(GraphRunError::msg(format!(
            "unknown server kind {other:?} (server {})",
            server.id
        ))),
    }
}

/// When `inherit_host_env` is true (local tasks), start from the graph_run process environment so
/// `$HOME` and friends match this machine. When false (remote SSH tasks), start from an empty map
/// so `$HOME` in `[[commands]]` / `cwd` is expanded only by the **remote** shell after `bash -l`
/// sets the usual login variables.
fn merged_task_env(
    server: &Server,
    shell: &Shell,
    cmd: &CmdDef,
    task: &Task,
    workspace_root: Option<&Path>,
    extra_env: &[(String, String)],
    inherit_host_env: bool,
) -> HashMap<String, String> {
    let base: HashMap<String, String> = if inherit_host_env {
        std::env::vars().collect()
    } else {
        HashMap::new()
    };
    let mut env = merge_entries(base, &shell.env);
    env = merge_entries(env, &cmd.env);
    env = merge_entries(env, &task.env);
    if let Some(root) = workspace_root {
        env.insert(
            "GRAPH_RUN_WORKSPACE".into(),
            root.to_string_lossy().into_owned(),
        );
        env.insert(
            "GRAPH_RUN_TMP".into(),
            root.join("tmp").to_string_lossy().into_owned(),
        );
    }
    for (k, v) in server.graph_run_env_entries() {
        env.insert(k, v);
    }
    if let Some(pw) = server.resolved_password() {
        env.insert("GRAPH_RUN_SERVER_PASSWORD".into(), pw);
    }
    for (k, v) in extra_env {
        env.insert(k.clone(), v.clone());
    }
    env
}

fn timeout_secs(task: &Task, cmd: &CmdDef, shell: &Shell, server: &Server) -> Option<u64> {
    [task.timeout, cmd.timeout, shell.timeout, server.timeout]
        .into_iter()
        .flatten()
        .min()
}

#[cfg(unix)]
fn run_remote(
    server: &Server,
    shell: &Shell,
    cmd: &CmdDef,
    task: &Task,
    workspace_root: Option<&Path>,
    extra_env: &[(String, String)],
) -> Result<std::process::ExitStatus> {
    use std::io;

    use ssh2::ExtendedData;

    let env = merged_task_env(
        server,
        shell,
        cmd,
        task,
        workspace_root,
        extra_env,
        false,
    );
    let timeout_secs = timeout_secs(task, cmd, shell, server);
    let timeout_ms: u32 = match timeout_secs {
        Some(s) => (s.saturating_mul(1000)).min(u32::MAX as u64) as u32,
        None => 300_000,
    };
    if timeout_ms == 0 {
        return Err(GraphRunError::msg(format!(
            "task {}: effective timeout is 0s for remote execution",
            task.id
        )));
    }

    let mut inner = String::new();
    let mut keys: Vec<&String> = env.keys().collect();
    keys.sort();
    for k in keys {
        if !is_exportable_env_name(k) {
            continue;
        }
        let v = &env[k];
        inner.push_str("export ");
        inner.push_str(k);
        inner.push('=');
        inner.push_str(&shell_single_quoted(v));
        inner.push(';');
    }
    if let Some(dir) = cmd.cwd.as_deref() {
        inner.push_str("cd ");
        inner.push_str(&shell_single_quoted(dir));
        inner.push_str(" && ");
    }
    inner.push_str(&cmd.command);

    let mut remote_line = String::new();
    remote_line.push_str(&shell.program);
    for a in &shell.args {
        remote_line.push(' ');
        remote_line.push_str(a);
    }
    remote_line.push(' ');
    remote_line.push_str(&shell_single_quoted(&inner));

    let sess = crate::transfer::ssh_connect_session(server, timeout_ms)?;
    let mut channel = sess.channel_session().map_err(ssh_channel_err)?;
    channel
        .handle_extended_data(ExtendedData::Merge)
        .map_err(ssh_channel_err)?;
    channel.exec(&remote_line).map_err(ssh_channel_err)?;

    io::copy(&mut channel, &mut io::stdout())
        .map_err(|e| GraphRunError::msg(format!("SSH channel output: {e}")))?;

    channel.wait_close().map_err(ssh_channel_err)?;
    let code = channel.exit_status().map_err(ssh_channel_err)?;
    Ok(exit_status_from_ssh_code(code))
}

#[cfg(unix)]
fn ssh_channel_err(e: ssh2::Error) -> GraphRunError {
    GraphRunError::msg(format!("SSH: {e}"))
}

#[cfg(unix)]
fn is_exportable_env_name(name: &str) -> bool {
    let mut it = name.chars();
    let Some(first) = it.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    it.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Single-quote for POSIX `sh` / `bash`, safe for `export NAME='...'`.
#[cfg(unix)]
fn shell_single_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

#[cfg(unix)]
fn exit_status_from_ssh_code(code: i32) -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(code << 8)
}

fn run_local(
    server: &Server,
    shell: &Shell,
    cmd: &CmdDef,
    task: &Task,
    workspace_root: Option<&Path>,
    extra_env: &[(String, String)],
) -> Result<std::process::ExitStatus> {
    let env = merged_task_env(
        server,
        shell,
        cmd,
        task,
        workspace_root,
        extra_env,
        true,
    );

    let cwd = cmd
        .cwd
        .as_deref()
        .or(Some("."))
        .map(Path::new);

    let timeout_secs = timeout_secs(task, cmd, shell, server);

    let mut command = Command::new(&shell.program);
    command
        .args(&shell.args)
        .arg(&cmd.command)
        .env_clear()
        .envs(&env)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    if let Some(dir) = cwd {
        command.current_dir(dir);
    }

    let mut child = command.spawn().map_err(|source| GraphRunError::Io {
        file: Path::new(&shell.program).to_path_buf(),
        source,
    })?;

    match timeout_secs {
        None => child.wait().map_err(|source| GraphRunError::Io {
            file: Path::new(".").to_path_buf(),
            source,
        }),
        Some(secs) => match child
            .wait_timeout(Duration::from_secs(secs))
            .map_err(|source| GraphRunError::Io {
                file: Path::new(".").to_path_buf(),
                source,
            })? {
            Some(status) => Ok(status),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                Err(GraphRunError::msg(format!(
                    "command timed out after {secs}s (task {})",
                    task.id
                )))
            }
        },
    }
}
