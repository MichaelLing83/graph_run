//! TOML config types. Many fields are reserved for future remote execution and UX.
#![allow(dead_code)]

use std::collections::HashMap;
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

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    #[default]
    Task,
    Start,
    End,
    Abort,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowNode {
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: NodeKind,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowEdge {
    pub from: String,
    pub to: String,
    pub failure: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowFile {
    #[serde(default)]
    pub nodes: Vec<WorkflowNode>,
    #[serde(default)]
    pub edges: Vec<WorkflowEdge>,
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
    let workflow: WorkflowFile = read_toml_path(workflow_path)?;

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
