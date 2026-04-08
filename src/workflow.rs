use std::collections::{HashMap, HashSet};
use std::path::Path;

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

        for n in self.nodes.values() {
            if !matches!(n.kind, NodeKind::Loop) {
                continue;
            }
            n.count.ok_or_else(|| {
                GraphRunError::msg(format!(
                    "workflow node {:?} has type \"loop\" but no count field",
                    n.id
                ))
            })?;
            let body = n.body.as_deref().ok_or_else(|| {
                GraphRunError::msg(format!(
                    "workflow node {:?} has type \"loop\" but no body field (workflow id of a task node)",
                    n.id
                ))
            })?;
            let b = self.nodes.get(body).ok_or_else(|| {
                GraphRunError::msg(format!(
                    "loop node {:?} body {:?} is not a workflow node",
                    n.id, body
                ))
            })?;
            if !matches!(b.kind, NodeKind::Task) {
                return Err(GraphRunError::msg(format!(
                    "loop node {:?} body {:?} must be a task node, not {:?}",
                    n.id, body, b.kind
                )));
            }
            self.success.get(&n.id).ok_or_else(|| {
                GraphRunError::msg(format!(
                    "loop node {:?} has no outgoing success [[edges]] row",
                    n.id
                ))
            })?;
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
                    "task or loop on node {from:?} failed and no failure edge is defined"
                ))
            })
    }

    /// Directed cycle in the **success** edge graph (`from -> to` only). If present, an execution
    /// where every task succeeds can run forever.
    fn find_success_edge_cycle(&self) -> Option<Vec<String>> {
        let mut verts = HashSet::new();
        for (a, b) in &self.success {
            verts.insert(a.as_str());
            verts.insert(b.as_str());
        }
        if verts.is_empty() {
            return None;
        }

        let verts: Vec<String> = verts.into_iter().map(String::from).collect();
        let mut color: HashMap<String, u8> = verts.iter().cloned().map(|v| (v, 0)).collect();

        for start in verts {
            if color[&start] != 0 {
                continue;
            }
            let mut stack: Vec<String> = Vec::new();
            if let Some(cycle) = Self::dfs_success_cycle(&start, &self.success, &mut color, &mut stack)
            {
                return Some(cycle);
            }
        }
        None
    }

    /// DFS colors: 0 white, 1 gray, 2 black.
    fn dfs_success_cycle(
        u: &str,
        success: &HashMap<String, String>,
        color: &mut HashMap<String, u8>,
        stack: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        *color.get_mut(u).expect("vertex in color map") = 1;
        stack.push(u.to_string());

        if let Some(v) = success.get(u) {
            let v_state = *color.get(v).unwrap_or(&0);
            match v_state {
                1 => {
                    let i = stack.iter().position(|n| n == v).expect("gray node on stack");
                    return Some(stack[i..].to_vec());
                }
                0 => {
                    if let Some(c) = Self::dfs_success_cycle(v, success, color, stack) {
                        return Some(c);
                    }
                }
                _ => {}
            }
        }

        stack.pop();
        *color.get_mut(u).expect("vertex in color map") = 2;
        None
    }
}

pub fn run_workflow(
    bundle: &ConfigBundle,
    mut workspace: Option<&mut Workspace>,
    allow_endless_loop: bool,
) -> Result<()> {
    let graph = TaskGraph::build(&bundle.workflow)?;

    if !allow_endless_loop {
        if let Some(cycle) = graph.find_success_edge_cycle() {
            let path = cycle.join(" -> ");
            return Err(GraphRunError::msg(format!(
                "workflow contains a directed cycle along success (non-failure) transitions: {path}. \
                 While every task succeeds, execution would never reach an end node. \
                 If this is intentional, re-run with --allow-endless-loop."
            )));
        }
    }

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
        if matches!(node.kind, NodeKind::Loop) {
            if let Some(body_id) = node.body.as_ref() {
                if !bundle.tasks.contains_key(body_id) {
                    return Err(GraphRunError::msg(format!(
                        "loop node {:?} body {:?} has no matching [[tasks]] entry in tasks file",
                        id, body_id
                    )));
                }
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
                let status = execute_task_by_node_id(
                    &graph,
                    bundle,
                    &node.id,
                    &mut workspace,
                    ws_root.as_deref(),
                    &[],
                )?;
                if status.success() {
                    current = graph.next_on_success(&current)?;
                } else {
                    current = graph.next_on_failure(&current)?;
                }
            }
            NodeKind::Loop => {
                let loop_id = node.id.as_str();
                let count = node.count.expect("loop validated with count");
                let body_id = node.body.as_deref().expect("loop validated with body");
                logging::record(
                    &mut workspace,
                    Level::Info,
                    format!("loop node id={loop_id} count={count} body={body_id}"),
                )?;
                let mut failed = false;
                for iter_idx in 0..count {
                    let extra = vec![
                        ("GRAPH_RUN_LOOP_INDEX".into(), iter_idx.to_string()),
                        ("GRAPH_RUN_LOOP_COUNT".into(), count.to_string()),
                        (
                            "GRAPH_RUN_LOOP_ITERATION".into(),
                            (iter_idx + 1).to_string(),
                        ),
                        ("GRAPH_RUN_LOOP_NODE_ID".into(), loop_id.to_string()),
                        ("GRAPH_RUN_LOOP_BODY_ID".into(), body_id.to_string()),
                    ];
                    logging::record(
                        &mut workspace,
                        Level::Info,
                        format!(
                            "loop {loop_id}: iteration {} of {count}",
                            iter_idx + 1
                        ),
                    )?;
                    let status = execute_task_by_node_id(
                        &graph,
                        bundle,
                        body_id,
                        &mut workspace,
                        ws_root.as_deref(),
                        &extra,
                    )?;
                    if !status.success() {
                        failed = true;
                        break;
                    }
                }
                if failed {
                    current = graph.next_on_failure(loop_id)?;
                } else {
                    current = graph.next_on_success(loop_id)?;
                }
            }
        }
    }
}

fn execute_task_by_node_id(
    graph: &TaskGraph,
    bundle: &ConfigBundle,
    task_node_id: &str,
    workspace: &mut Option<&mut Workspace>,
    ws_root: Option<&Path>,
    extra_env: &[(String, String)],
) -> Result<std::process::ExitStatus> {
    let node = graph.nodes.get(task_node_id).ok_or_else(|| {
        GraphRunError::msg(format!("missing workflow node {task_node_id:?}"))
    })?;
    if !matches!(node.kind, NodeKind::Task) {
        return Err(GraphRunError::msg(format!(
            "workflow node {task_node_id:?} has kind {:?}, expected task",
            node.kind
        )));
    }
    let task = bundle
        .tasks
        .get(task_node_id)
        .ok_or_else(|| GraphRunError::msg(format!("unknown task {task_node_id:?}")))?;
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
    logging::record(workspace, Level::Info, task_header)?;
    logging::record(workspace, Level::Debug, task_cmd)?;
    let status = execute::run_task(
        resolved.server,
        resolved.shell,
        resolved.command,
        task,
        ws_root,
        extra_env,
    )?;
    logging::record(
        workspace,
        Level::Info,
        format!(
            "task id={} finished success={} code={:?}",
            task.id,
            status.success(),
            status.code()
        ),
    )?;
    Ok(status)
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
