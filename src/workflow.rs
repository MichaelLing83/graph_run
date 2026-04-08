use std::collections::HashMap;

use crate::config::{
    Command, ConfigBundle, NodeKind, Server, Shell, Task, WorkflowFile, WorkflowNode,
};
use log::Level;

use crate::error::{GraphRunError, Result};
use crate::execute;
use crate::logging;
use crate::workspace::Workspace;

struct TaskGraph {
    nodes: HashMap<String, WorkflowNode>,
    success: HashMap<String, String>,
    failure: HashMap<String, String>,
}

impl TaskGraph {
    fn build(wf: &WorkflowFile) -> Result<Self> {
        let mut nodes = HashMap::new();
        for n in &wf.nodes {
            if nodes.insert(n.id.clone(), n.clone()).is_some() {
                return Err(GraphRunError::msg(format!("duplicate workflow node id {:?}", n.id)));
            }
        }

        let mut success = HashMap::new();
        let mut failure = HashMap::new();
        for e in &wf.edges {
            if success
                .insert(e.from.clone(), e.to.clone())
                .is_some()
            {
                return Err(GraphRunError::msg(format!(
                    "duplicate success edge from {:?}",
                    e.from
                )));
            }
            if let Some(f) = &e.failure {
                if failure.insert(e.from.clone(), f.clone()).is_some() {
                    return Err(GraphRunError::msg(format!(
                        "duplicate failure edge from {:?}",
                        e.from
                    )));
                }
            }
        }

        let g = TaskGraph {
            nodes,
            success,
            failure,
        };
        g.validate()?;
        Ok(g)
    }

    fn validate(&self) -> Result<()> {
        for id in self.nodes.keys() {
            for next in self
                .success
                .get(id)
                .into_iter()
                .chain(self.failure.get(id).into_iter())
            {
                if !self.nodes.contains_key(next) {
                    return Err(GraphRunError::msg(format!(
                        "edge references missing node {:?} (from {:?})",
                        next, id
                    )));
                }
            }
        }

        let starts: Vec<_> = self
            .nodes
            .values()
            .filter(|n| matches!(n.kind, NodeKind::Start))
            .collect();
        match starts.len() {
            0 => return Err(GraphRunError::msg("workflow has no start node")),
            1 => {}
            _ => return Err(GraphRunError::msg("workflow has more than one start node")),
        }

        Ok(())
    }

    fn start_id(&self) -> Result<String> {
        self.nodes
            .values()
            .find(|n| matches!(n.kind, NodeKind::Start))
            .map(|n| n.id.clone())
            .ok_or_else(|| GraphRunError::msg("workflow has no start node"))
    }

    fn next_on_success(&self, from: &str) -> Result<String> {
        self.success
            .get(from)
            .cloned()
            .ok_or_else(|| GraphRunError::msg(format!("no outgoing success edge from {from:?}")))
    }

    fn next_on_failure(&self, from: &str) -> Result<String> {
        self.failure
            .get(from)
            .cloned()
            .ok_or_else(|| {
                GraphRunError::msg(format!(
                    "task on node {from:?} failed and no failure edge is defined"
                ))
            })
    }
}

pub fn run_workflow(bundle: &ConfigBundle, mut workspace: Option<&mut Workspace>) -> Result<()> {
    let graph = TaskGraph::build(&bundle.workflow)?;

    let ws_root = workspace.as_ref().map(|w| w.root().to_path_buf());
    let log_file_note = workspace
        .as_ref()
        .map(|w| w.log_file_path().display().to_string())
        .unwrap_or_default();
    logging::record(
        &mut workspace,
        Level::Debug,
        format!("graph_run: start log_file={log_file_note}"),
    )?;
    logging::record(
        &mut workspace,
        Level::Info,
        "graph_run: workflow execution started",
    )?;

    for (id, node) in &graph.nodes {
        if matches!(node.kind, NodeKind::Task) {
            if !bundle.tasks.contains_key(id) {
                return Err(GraphRunError::msg(format!(
                    "workflow task node {:?} has no matching [[tasks]] entry in tasks file",
                    id
                )));
            }
        }
    }

    let mut current = graph.start_id()?;
    loop {
        let node = graph
            .nodes
            .get(&current)
            .ok_or_else(|| GraphRunError::msg(format!("missing node {current:?}")))?;

        match node.kind {
            NodeKind::End => {
                logging::record(
                    &mut workspace,
                    Level::Info,
                    "graph_run: reached end node (success)",
                )?;
                return Ok(());
            }
            NodeKind::Abort => {
                let _ = logging::record(
                    &mut workspace,
                    Level::Warn,
                    "graph_run: reached abort node (failure branch)",
                );
                return Err(GraphRunError::msg(
                    "workflow finished at abort (failure branch)",
                ));
            }
            NodeKind::Start => {
                current = graph.next_on_success(&current)?;
            }
            NodeKind::Task => {
                let task = bundle
                    .tasks
                    .get(&node.id)
                    .ok_or_else(|| GraphRunError::msg(format!("unknown task {:?}", node.id)))?;
                let resolved = resolve_task(bundle, task)?;
                let task_header = format!(
                    "task id={} server={} shell={} command_id={} shell_invocation={} {}",
                    task.id,
                    task.server_id,
                    task.shell_id,
                    task.command_id,
                    resolved.shell.program,
                    resolved.shell.args.join(" ")
                );
                let task_cmd = format!("  run: {}", resolved.command.command);
                logging::record(&mut workspace, Level::Info, task_header)?;
                logging::record(&mut workspace, Level::Debug, task_cmd)?;
                let status = execute::run_task(
                    resolved.server,
                    resolved.shell,
                    resolved.command,
                    task,
                    ws_root.as_deref(),
                )?;
                logging::record(
                    &mut workspace,
                    Level::Info,
                    format!(
                        "task id={} finished success={} code={:?}",
                        task.id,
                        status.success(),
                        status.code()
                    ),
                )?;

                if status.success() {
                    current = graph.next_on_success(&current)?;
                } else {
                    current = graph.next_on_failure(&current)?;
                }
            }
        }
    }
}

struct Resolved<'a> {
    server: &'a Server,
    shell: &'a Shell,
    command: &'a Command,
}

fn resolve_task<'a>(bundle: &'a ConfigBundle, task: &'a Task) -> Result<Resolved<'a>> {
    let server = bundle.servers.get(&task.server_id).ok_or_else(|| {
        GraphRunError::msg(format!(
            "task {:?} references unknown server {:?}",
            task.id, task.server_id
        ))
    })?;
    let shell = bundle.shells.get(&task.shell_id).ok_or_else(|| {
        GraphRunError::msg(format!(
            "task {:?} references unknown shell {:?}",
            task.id, task.shell_id
        ))
    })?;
    let command = bundle.commands.get(&task.command_id).ok_or_else(|| {
        GraphRunError::msg(format!(
            "task {:?} references unknown command {:?}",
            task.id, task.command_id
        ))
    })?;
    Ok(Resolved {
        server,
        shell,
        command,
    })
}
