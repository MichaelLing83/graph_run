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
        other => Err(GraphRunError::msg(format!(
            "remote server kind {other:?} is not implemented yet (server {})",
            server.id
        ))),
    }
}

fn run_local(
    server: &Server,
    shell: &Shell,
    cmd: &CmdDef,
    task: &Task,
    workspace_root: Option<&Path>,
    extra_env: &[(String, String)],
) -> Result<std::process::ExitStatus> {
    let base: HashMap<String, String> = std::env::vars().collect();
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

    let cwd = cmd
        .cwd
        .as_deref()
        .or(Some("."))
        .map(Path::new);

    let timeout_secs = [
        task.timeout,
        cmd.timeout,
        shell.timeout,
        server.timeout,
    ]
    .into_iter()
    .flatten()
    .min();

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
