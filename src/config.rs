//! TOML config types. Many fields are reserved for future remote execution and UX.
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::error::{GraphRunError, Result};

fn read_toml_path<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let file = path.to_path_buf();
    let text = fs::read_to_string(path).map_err(|source| GraphRunError::Io {
        file: file.clone(),
        source,
    })?;
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

#[derive(Debug, Clone, Deserialize)]
pub struct Task {
    pub id: String,
    pub server_id: String,
    pub shell_id: String,
    pub command_id: String,
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
) -> Result<ConfigBundle> {
    let servers_root: ServersRoot = read_toml_path(servers_path)?;
    let shells_root: ShellsRoot = read_toml_path(shells_path)?;
    let commands_root: CommandsRoot = read_toml_path(commands_path)?;
    let tasks_root: TasksRoot = read_toml_path(tasks_path)?;
    let mut workflow: WorkflowFile = read_toml_path(workflow_path)?;
    workflow.ensure_default_control_nodes();

    let servers = index_by_id(servers_root.servers, |s| s.id.clone(), servers_path)?;
    let shells = index_by_id(shells_root.shells, |s| s.id.clone(), shells_path)?;
    let commands = index_by_id(commands_root.commands, |c| c.id.clone(), commands_path)?;
    let tasks = index_by_id(tasks_root.tasks, |t| t.id.clone(), tasks_path)?;

    Ok(ConfigBundle {
        servers,
        shells,
        commands,
        tasks,
        workflow,
    })
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
