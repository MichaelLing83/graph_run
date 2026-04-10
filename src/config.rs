//! TOML config types. Many fields are reserved for future remote execution and UX.
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::error::{GraphRunError, Result};

fn read_toml_path<T: serde::de::DeserializeOwned>(
    path: &Path,
    constants: Option<&HashMap<String, String>>,
) -> Result<T> {
    let file = path.to_path_buf();
    let text = fs::read_to_string(path).map_err(|source| GraphRunError::Io {
        file: file.clone(),
        source,
    })?;
    let text = match constants {
        Some(map) => crate::constants::expand_template(&text, map, path)?,
        None => text,
    };
    toml::from_str(&text).map_err(|source| GraphRunError::Toml { file, source })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnvStrategy {
    Override,
    Prepend,
    Append,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EnvEntry {
    pub name: String,
    pub strategy: EnvStrategy,
    pub value: String,
    pub separator: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Server {
    pub id: String,
    pub kind: String,
    pub description: Option<String>,
    pub transport: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub timeout: Option<u64>,
    /// Optional password from TOML, passed to the child as `GRAPH_RUN_SERVER_PASSWORD`.
    /// Prefer **`password_env`** or SSH keys for anything you might commit; empty string is ignored.
    #[serde(default)]
    pub password: Option<String>,
    /// Name of an environment variable **in the graph_run process** whose value is copied into the
    /// child as `GRAPH_RUN_SERVER_PASSWORD`. If this name is set in the environment (even to an
    /// empty string), that value wins over **`password`**. Prefer SSH keys over passwords when possible.
    #[serde(default)]
    pub password_env: Option<String>,
}

impl Server {
    /// Environment entries derived from this server for every task that uses it (host, user, etc.).
    /// Empty strings mean the field was unset in TOML.
    pub fn graph_run_env_entries(&self) -> Vec<(String, String)> {
        let mut out = vec![
            ("GRAPH_RUN_SERVER_ID".into(), self.id.clone()),
            ("GRAPH_RUN_SERVER_KIND".into(), self.kind.clone()),
            (
                "GRAPH_RUN_SERVER_TRANSPORT".into(),
                self.transport.clone().unwrap_or_default(),
            ),
            ("GRAPH_RUN_SERVER_HOST".into(), self.host.clone().unwrap_or_default()),
            (
                "GRAPH_RUN_SERVER_PORT".into(),
                self.port.map(|p| p.to_string()).unwrap_or_default(),
            ),
            ("GRAPH_RUN_SERVER_USER".into(), self.user.clone().unwrap_or_default()),
            (
                "GRAPH_RUN_SERVER_DESCRIPTION".into(),
                self.description.clone().unwrap_or_default(),
            ),
        ];
        let userhost = match (&self.user, &self.host) {
            (Some(u), Some(h)) => format!("{u}@{h}"),
            _ => String::new(),
        };
        out.push(("GRAPH_RUN_SSH_USERHOST".into(), userhost));
        out
    }

    /// Value for `GRAPH_RUN_SERVER_PASSWORD` when running a task on this server.
    ///
    /// If **`password_env`** is set and that variable is present in the `graph_run` process, its
    /// value is used (including empty). Otherwise a non-empty **`password`** from TOML is used.
    pub fn resolved_password(&self) -> Option<String> {
        if let Some(pname) = self.password_env.as_deref() {
            if let Ok(pw) = std::env::var(pname) {
                return Some(pw);
            }
        }
        self.password.as_ref().filter(|s| !s.is_empty()).cloned()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Shell {
    pub id: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub description: Option<String>,
    pub timeout: Option<u64>,
    #[serde(default)]
    pub env: Vec<EnvEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Command {
    pub id: String,
    pub command: String,
    pub description: Option<String>,
    pub cwd: Option<String>,
    pub timeout: Option<u64>,
    #[serde(default)]
    pub env: Vec<EnvEntry>,
}

/// When set on a [[tasks]] row, `graph_run` copies `source_path` → `dest_path` using SFTP-like
/// semantics (mode and mtime preserved) instead of running a shell command.
#[derive(Debug, Clone, Deserialize)]
pub struct TransferSpec {
    pub source_server_id: String,
    pub dest_server_id: String,
    /// Path on the source server. A trailing `/` means “copy directory contents” (like `rsync`).
    pub source_path: String,
    /// Path on the destination server. A trailing `/` is accepted; remote paths use POSIX `/`.
    pub dest_path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Task {
    pub id: String,
    #[serde(default)]
    pub transfer: Option<TransferSpec>,
    #[serde(default)]
    pub server_id: Option<String>,
    #[serde(default)]
    pub shell_id: Option<String>,
    #[serde(default)]
    pub command_id: Option<String>,
    pub description: Option<String>,
    pub timeout: Option<u64>,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    #[default]
    Task,
    Start,
    End,
    Abort,
    /// Counted subgraph: success edges from this node enter the body; matching `loop_end` closes each pass.
    Loop,
    /// Marks the end of one pass through a loop body; `loop` field names the `loop` node id.
    #[serde(rename = "loop_end")]
    LoopEnd,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowNode {
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: NodeKind,
    pub name: Option<String>,
    /// Loop iterations (`type = "loop"`). Zero means the body is never entered.
    #[serde(default)]
    pub count: Option<u32>,
    /// For `type = "loop_end"`: id of the `loop` node this marker closes.
    #[serde(default, rename = "loop")]
    pub ends_loop: Option<String>,
}

fn default_edge_failure() -> String {
    "abort".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowEdge {
    pub from: String,
    pub to: String,
    /// On task (or other runnable) failure at `from`, transit to this node. Defaults to **`abort`**.
    #[serde(default = "default_edge_failure")]
    pub failure: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowFile {
    #[serde(default)]
    pub nodes: Vec<WorkflowNode>,
    #[serde(default)]
    pub edges: Vec<WorkflowEdge>,
}

impl WorkflowFile {
    /// If `[[nodes]]` for `start`, `end`, or `abort` are omitted, append the standard control nodes
    /// (`type = "start" | "end" | "abort"`). Existing definitions are left unchanged so names and
    /// future fields can still be set in TOML.
    pub fn ensure_default_control_nodes(&mut self) {
        let present: HashSet<String> = self.nodes.iter().map(|n| n.id.clone()).collect();
        if !present.contains("start") {
            self.nodes.push(WorkflowNode {
                id: "start".into(),
                kind: NodeKind::Start,
                name: None,
                count: None,
                ends_loop: None,
            });
        }
        if !present.contains("end") {
            self.nodes.push(WorkflowNode {
                id: "end".into(),
                kind: NodeKind::End,
                name: None,
                count: None,
                ends_loop: None,
            });
        }
        if !present.contains("abort") {
            self.nodes.push(WorkflowNode {
                id: "abort".into(),
                kind: NodeKind::Abort,
                name: None,
                count: None,
                ends_loop: None,
            });
        }
    }
}

#[derive(Debug)]
pub struct ConfigBundle {
    pub servers: HashMap<String, Server>,
    pub shells: HashMap<String, Shell>,
    pub commands: HashMap<String, Command>,
    pub tasks: HashMap<String, Task>,
    pub workflow: WorkflowFile,
}

#[derive(Debug, Deserialize)]
struct ServersRoot {
    #[serde(default)]
    servers: Vec<Server>,
}

#[derive(Debug, Deserialize)]
struct ShellsRoot {
    #[serde(default)]
    shells: Vec<Shell>,
}

#[derive(Debug, Deserialize)]
struct CommandsRoot {
    #[serde(default)]
    commands: Vec<Command>,
}

#[derive(Debug, Deserialize)]
struct TasksRoot {
    #[serde(default)]
    tasks: Vec<Task>,
}

pub fn load_bundle(
    servers_path: &Path,
    shells_path: &Path,
    commands_path: &Path,
    tasks_path: &Path,
    workflow_path: &Path,
    constants_path: Option<&Path>,
) -> Result<ConfigBundle> {
    let constants = match constants_path {
        Some(p) => Some(crate::constants::load_constants_file(p)?),
        None => None,
    };
    let cref = constants.as_ref();
    let servers_root: ServersRoot = read_toml_path(servers_path, cref)?;
    let shells_root: ShellsRoot = read_toml_path(shells_path, cref)?;
    let commands_root: CommandsRoot = read_toml_path(commands_path, cref)?;
    let tasks_root: TasksRoot = read_toml_path(tasks_path, cref)?;
    let mut workflow: WorkflowFile = read_toml_path(workflow_path, cref)?;
    workflow.ensure_default_control_nodes();

    let servers = index_by_id(servers_root.servers, |s| s.id.clone(), servers_path)?;
    let shells = index_by_id(shells_root.shells, |s| s.id.clone(), shells_path)?;
    let commands = index_by_id(commands_root.commands, |c| c.id.clone(), commands_path)?;
    for task in &tasks_root.tasks {
        validate_task_definition(task)?;
    }
    let tasks = index_by_id(tasks_root.tasks, |t| t.id.clone(), tasks_path)?;

    Ok(ConfigBundle {
        servers,
        shells,
        commands,
        tasks,
        workflow,
    })
}

#[cfg(test)]
mod server_env_tests {
    use super::Server;

    #[test]
    fn graph_run_env_local_server() {
        let s = Server {
            id: "local".into(),
            kind: "local".into(),
            description: None,
            transport: None,
            host: None,
            port: None,
            user: None,
            timeout: None,
            password: None,
            password_env: None,
        };
        let m: std::collections::HashMap<_, _> = s.graph_run_env_entries().into_iter().collect();
        assert_eq!(m["GRAPH_RUN_SERVER_ID"], "local");
        assert_eq!(m["GRAPH_RUN_SERVER_KIND"], "local");
        assert_eq!(m["GRAPH_RUN_SSH_USERHOST"], "");
    }

    #[test]
    fn graph_run_env_remote_ssh_userhost() {
        let s = Server {
            id: "r".into(),
            kind: "remote".into(),
            description: None,
            transport: Some("ssh".into()),
            host: Some("10.0.0.5".into()),
            port: Some(2222),
            user: Some("deploy".into()),
            timeout: None,
            password: None,
            password_env: None,
        };
        let m: std::collections::HashMap<_, _> = s.graph_run_env_entries().into_iter().collect();
        assert_eq!(m["GRAPH_RUN_SERVER_HOST"], "10.0.0.5");
        assert_eq!(m["GRAPH_RUN_SERVER_PORT"], "2222");
        assert_eq!(m["GRAPH_RUN_SERVER_USER"], "deploy");
        assert_eq!(m["GRAPH_RUN_SSH_USERHOST"], "deploy@10.0.0.5");
    }

    fn sample_server(password: Option<String>, password_env: Option<String>) -> Server {
        Server {
            id: "s".into(),
            kind: "remote".into(),
            description: None,
            transport: None,
            host: None,
            port: None,
            user: None,
            timeout: None,
            password,
            password_env,
        }
    }

    #[test]
    fn resolved_password_from_toml() {
        let s = sample_server(Some("pw-toml".into()), None);
        assert_eq!(s.resolved_password().as_deref(), Some("pw-toml"));
    }

    #[test]
    fn resolved_password_empty_toml_ignored() {
        let s = sample_server(Some(String::new()), None);
        assert_eq!(s.resolved_password(), None);
    }

    #[test]
    fn resolved_password_env_overrides_toml() {
        const KEY: &str = "GRAPH_RUN_TEST_SERVER_PW_OVERRIDE";
        std::env::set_var(KEY, "pw-env");
        let s = sample_server(Some("pw-toml".into()), Some(KEY.into()));
        assert_eq!(s.resolved_password().as_deref(), Some("pw-env"));
        std::env::remove_var(KEY);
    }

    #[test]
    fn resolved_password_falls_back_when_env_unset() {
        let s = sample_server(
            Some("pw-toml".into()),
            Some("GRAPH_RUN_TEST_SERVER_PW_DOES_NOT_EXIST".into()),
        );
        assert_eq!(s.resolved_password().as_deref(), Some("pw-toml"));
    }
}

fn validate_task_definition(task: &Task) -> Result<()> {
    match &task.transfer {
        Some(_) => {
            if task.server_id.is_some() || task.shell_id.is_some() || task.command_id.is_some() {
                return Err(GraphRunError::msg(format!(
                    "task {:?}: transfer tasks must not set server_id, shell_id, or command_id",
                    task.id
                )));
            }
            Ok(())
        }
        None => {
            if task.server_id.is_none() || task.shell_id.is_none() || task.command_id.is_none() {
                return Err(GraphRunError::msg(format!(
                    "task {:?}: command tasks require server_id, shell_id, and command_id",
                    task.id
                )));
            }
            Ok(())
        }
    }
}

fn index_by_id<T>(
    items: Vec<T>,
    id_fn: impl Fn(&T) -> String,
    path: &Path,
) -> Result<HashMap<String, T>> {
    let mut map = HashMap::new();
    for item in items {
        let id = id_fn(&item);
        if map.insert(id.clone(), item).is_some() {
            return Err(GraphRunError::msg(format!(
                "duplicate id {id:?} in {}",
                path.display()
            )));
        }
    }
    Ok(map)
}
