use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};

use crate::config::{
    Command, ConfigBundle, NodeKind, Server, Shell, Task, WorkflowFile, WorkflowNode,
};
use log::Level;

use crate::error::{GraphRunError, Result};
use crate::execute;
use crate::logging;
use crate::workspace::Workspace;

/// One active counted loop; `passes_done` counts completed traversals body → loop_end.
#[derive(Debug, Clone)]
struct LoopFrame {
    loop_id: String,
    /// Successors of the loop node: parallel entry points into the body (re-dispatched each pass).
    body_targets: Vec<String>,
    loop_end_id: String,
    count: u32,
    passes_done: u32,
}

struct TaskGraph {
    nodes: HashMap<String, WorkflowNode>,
    /// Successors per node; multiple rows with the same `from` are merged (order preserved). Fan-out
    /// runs in parallel; fan-in uses a barrier when indegree > 1.
    success: HashMap<String, Vec<String>>,
    failure: HashMap<String, String>,
    join_barriers: HashMap<String, Arc<JoinGate>>,
}

/// Synchronizes parallel branches at a node with indegree > 1; `abort` wakes waiters if any branch
/// fails so threads do not deadlock at the gate.
struct JoinGate {
    needed: usize,
    inner: Mutex<JoinGateState>,
    cvar: Condvar,
}

struct JoinGateState {
    waiting: usize,
    generation: u64,
    aborted: bool,
}

impl JoinGate {
    fn new(needed: usize) -> Self {
        Self {
            needed,
            inner: Mutex::new(JoinGateState {
                waiting: 0,
                generation: 0,
                aborted: false,
            }),
            cvar: Condvar::new(),
        }
    }

    /// `Ok(true)` = this thread runs the join node; `Ok(false)` = follower, exit branch.
    fn wait(&self) -> Result<bool> {
        let mut s = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if s.aborted {
            return Err(GraphRunError::msg(
                "parallel workflow aborted (another branch failed)",
            ));
        }
        s.waiting += 1;
        if s.waiting == self.needed {
            s.waiting = 0;
            s.generation = s.generation.wrapping_add(1);
            self.cvar.notify_all();
            return Ok(true);
        }
        let my_gen = s.generation;
        while s.generation == my_gen && !s.aborted {
            s = self
                .cvar
                .wait(s)
                .unwrap_or_else(|e| e.into_inner());
        }
        if s.aborted {
            return Err(GraphRunError::msg(
                "parallel workflow aborted (another branch failed)",
            ));
        }
        Ok(false)
    }

    fn abort(&self) {
        let mut s = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        s.aborted = true;
        self.cvar.notify_all();
    }
}

impl TaskGraph {
    fn build(wf: &WorkflowFile) -> Result<Self> {
        let mut nodes = HashMap::new();
        for n in &wf.nodes {
            if nodes.insert(n.id.clone(), n.clone()).is_some() {
                return Err(GraphRunError::msg(format!("duplicate workflow node id {:?}", n.id)));
            }
        }

        let mut success: HashMap<String, Vec<String>> = HashMap::new();
        let mut failure = HashMap::new();
        for e in &wf.edges {
            let entry = success.entry(e.from.clone()).or_default();
            if entry.contains(&e.to) {
                return Err(GraphRunError::msg(format!(
                    "duplicate success edge from {:?} to {:?} (each target may appear at most once per from)",
                    e.from, e.to
                )));
            }
            entry.push(e.to.clone());
            if e.failure.is_empty() {
                return Err(GraphRunError::msg(format!(
                    "edge from {:?} to {:?} has empty failure target (omit the key for default \"abort\", or set failure = \"...\")",
                    e.from, e.to
                )));
            }
            match failure.get(&e.from) {
                None => {
                    failure.insert(e.from.clone(), e.failure.clone());
                }
                Some(prev) if prev == &e.failure => {}
                Some(prev) => {
                    return Err(GraphRunError::msg(format!(
                        "conflicting failure edges from {:?}: {:?} vs {:?}",
                        e.from, prev, e.failure
                    )));
                }
            }
        }

        let mut join_in_degree: HashMap<String, usize> = HashMap::new();
        for (_from, tos) in &success {
            for to in tos {
                *join_in_degree.entry(to.clone()).or_insert(0) += 1;
            }
        }

        let mut join_barriers = HashMap::new();
        for (id, &deg) in &join_in_degree {
            if deg > 1 {
                join_barriers.insert(id.clone(), Arc::new(JoinGate::new(deg)));
            }
        }

        let g = TaskGraph {
            nodes,
            success,
            failure,
            join_barriers,
        };
        g.validate()?;
        Ok(g)
    }

    fn loop_end_node_for(&self, loop_id: &str) -> Result<String> {
        let mut found: Option<String> = None;
        for n in self.nodes.values() {
            if !matches!(n.kind, NodeKind::LoopEnd) {
                continue;
            }
            if n.ends_loop.as_deref() == Some(loop_id) {
                if found.is_some() {
                    return Err(GraphRunError::msg(format!(
                        "multiple loop_end nodes close loop {loop_id:?}"
                    )));
                }
                found = Some(n.id.clone());
            }
        }
        found.ok_or_else(|| {
            GraphRunError::msg(format!(
                "loop {loop_id:?} has no matching loop_end node (use type \"loop_end\" and loop = \"{loop_id}\")"
            ))
        })
    }

    fn validate(&self) -> Result<()> {
        for id in self.nodes.keys() {
            for next in self
                .success
                .get(id)
                .into_iter()
                .flatten()
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
            if matches!(n.kind, NodeKind::LoopEnd) {
                let lid = n.ends_loop.as_deref().ok_or_else(|| {
                    GraphRunError::msg(format!(
                        "workflow node {:?} has type \"loop_end\" but no loop field (parent loop id)",
                        n.id
                    ))
                })?;
                let lp = self.nodes.get(lid).ok_or_else(|| {
                    GraphRunError::msg(format!(
                        "loop_end {:?} references unknown loop {:?}",
                        n.id, lid
                    ))
                })?;
                if !matches!(lp.kind, NodeKind::Loop) {
                    return Err(GraphRunError::msg(format!(
                        "loop_end {:?} loop field {:?} must name a node with type \"loop\"",
                        n.id, lid
                    )));
                }
                if self.success.get(&n.id).map(|v| v.is_empty()).unwrap_or(true) {
                    return Err(GraphRunError::msg(format!(
                        "loop_end {:?} must have at least one outgoing success [[edges]] row (continuation after the loop)",
                        n.id
                    )));
                }
            }
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
            let body_starts = self.success.get(&n.id).filter(|v| !v.is_empty()).ok_or_else(|| {
                GraphRunError::msg(format!(
                    "loop node {:?} has no outgoing success [[edges]] rows (these targets are the loop body entry point(s))",
                    n.id
                ))
            })?;
            for t in body_starts {
                let b = self.nodes.get(t).ok_or_else(|| {
                    GraphRunError::msg(format!(
                        "loop node {:?} success edge references missing node {:?}",
                        n.id, t
                    ))
                })?;
                if matches!(
                    b.kind,
                    NodeKind::Start | NodeKind::End | NodeKind::Abort | NodeKind::LoopEnd
                ) {
                    return Err(GraphRunError::msg(format!(
                        "loop node {:?} body entry {:?} cannot be {:?}",
                        n.id, t, b.kind
                    )));
                }
            }
            self.loop_end_node_for(&n.id)?;
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

    fn success_targets(&self, from: &str) -> Result<&[String]> {
        self.success
            .get(from)
            .filter(|v| !v.is_empty())
            .map(|v| v.as_slice())
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
        for (a, tos) in &self.success {
            verts.insert(a.as_str());
            for b in tos {
                verts.insert(b.as_str());
            }
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
            if let Some(cycle) =
                Self::dfs_success_cycle(&start, &self.success, &mut color, &mut stack)
            {
                return Some(cycle);
            }
        }
        None
    }

    /// DFS colors: 0 white, 1 gray, 2 black.
    fn dfs_success_cycle(
        u: &str,
        success: &HashMap<String, Vec<String>>,
        color: &mut HashMap<String, u8>,
        stack: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        *color.get_mut(u).expect("vertex in color map") = 1;
        stack.push(u.to_string());

        if let Some(tos) = success.get(u) {
            for v in tos {
                let v_state = *color.get(v).unwrap_or(&0);
                match v_state {
                    1 => {
                        let i = stack
                            .iter()
                            .position(|n| n == v)
                            .expect("gray node on stack");
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
        }

        stack.pop();
        *color.get_mut(u).expect("vertex in color map") = 2;
        None
    }
}

pub fn run_workflow(
    bundle: &ConfigBundle,
    workspace: Option<&mut Workspace>,
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
    let ws_log = workspace.as_ref().map(|w| &**w);
    logging::record(
        ws_log,
        Level::Debug,
        format!("graph_run: start log_file={log_file_note}"),
    )?;
    logging::record(
        ws_log,
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

    let shared_err = Mutex::new(None::<String>);
    let start = graph.start_id()?;
    run_from(
        &graph,
        bundle,
        start,
        Vec::new(),
        ws_log,
        ws_root.as_deref(),
        &shared_err,
    )?;
    if let Some(msg) = shared_err.into_inner().unwrap() {
        return Err(GraphRunError::msg(msg));
    }
    Ok(())
}

fn merge_err(graph: &TaskGraph, shared_err: &Mutex<Option<String>>, r: Result<()>) {
    if let Err(e) = r {
        let mut g = shared_err
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if g.is_none() {
            *g = Some(e.to_string());
            drop(g);
            for gate in graph.join_barriers.values() {
                gate.abort();
            }
        }
    }
}

fn check_parallel_abort(shared_err: &Mutex<Option<String>>) -> Result<()> {
    if let Some(msg) = shared_err
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
    {
        return Err(GraphRunError::msg(msg));
    }
    Ok(())
}

fn dispatch_successors(
    graph: &TaskGraph,
    bundle: &ConfigBundle,
    tos: &[String],
    loop_stack: Vec<LoopFrame>,
    workspace: Option<&Workspace>,
    ws_root: Option<&Path>,
    shared_err: &Mutex<Option<String>>,
) -> Result<()> {
    check_parallel_abort(shared_err)?;
    match tos {
        [] => Err(GraphRunError::msg("internal: empty successor list")),
        [one] => run_from(
            graph,
            bundle,
            one.clone(),
            loop_stack,
            workspace,
            ws_root,
            shared_err,
        ),
        many => {
            std::thread::scope(|s| {
                for to in many.iter().skip(1) {
                    let to = to.clone();
                    let ls = loop_stack.clone();
                    s.spawn(move || {
                        merge_err(
                            graph,
                            shared_err,
                            run_from(
                                graph,
                                bundle,
                                to,
                                ls,
                                workspace,
                                ws_root,
                                shared_err,
                            ),
                        );
                    });
                }
                merge_err(
                    graph,
                    shared_err,
                    run_from(
                        graph,
                        bundle,
                        many[0].clone(),
                        loop_stack,
                        workspace,
                        ws_root,
                        shared_err,
                    ),
                );
            });
            check_parallel_abort(shared_err)?;
            Ok(())
        }
    }
}

fn run_from(
    graph: &TaskGraph,
    bundle: &ConfigBundle,
    current: String,
    mut loop_stack: Vec<LoopFrame>,
    workspace: Option<&Workspace>,
    ws_root: Option<&Path>,
    shared_err: &Mutex<Option<String>>,
) -> Result<()> {
    check_parallel_abort(shared_err)?;

    if let Some(gate) = graph.join_barriers.get(&current) {
        match gate.wait() {
            Ok(true) => {}
            Ok(false) => return Ok(()),
            Err(e) => return Err(e),
        }
    }

    let node = graph
        .nodes
        .get(&current)
        .ok_or_else(|| GraphRunError::msg(format!("missing node {current:?}")))?;

    match node.kind {
        NodeKind::End => {
            if !loop_stack.is_empty() {
                return Err(GraphRunError::msg(format!(
                    "reached end node but {} loop(s) are still open (each loop needs a loop_end node)",
                    loop_stack.len()
                )));
            }
            logging::record(
                workspace,
                Level::Info,
                "graph_run: reached end node (success)",
            )?;
            Ok(())
        }
        NodeKind::Abort => {
            let _ = logging::record(
                workspace,
                Level::Warn,
                "graph_run: reached abort node (failure branch)",
            );
            Err(GraphRunError::msg(
                "workflow finished at abort (failure branch)",
            ))
        }
        NodeKind::Start => {
            let tos = graph.success_targets(&current)?;
            dispatch_successors(
                graph,
                bundle,
                tos,
                loop_stack,
                workspace,
                ws_root,
                shared_err,
            )
        }
        NodeKind::Task => {
            let extra = loop_env_for_stack(&loop_stack);
            let status = execute_task_by_node_id(
                graph,
                bundle,
                &node.id,
                workspace,
                ws_root,
                &extra,
            )?;
            if status.success() {
                let tos = graph.success_targets(&current)?;
                dispatch_successors(
                    graph,
                    bundle,
                    tos,
                    loop_stack,
                    workspace,
                    ws_root,
                    shared_err,
                )
            } else {
                let fail_to = graph.next_on_failure(&current)?;
                run_from(
                    graph,
                    bundle,
                    fail_to,
                    Vec::new(),
                    workspace,
                    ws_root,
                    shared_err,
                )
            }
        }
        NodeKind::Loop => {
            let loop_id = node.id.clone();
            let count = node.count.expect("loop validated with count");
            let body_targets: Vec<String> = graph.success_targets(&loop_id)?.to_vec();
            let loop_end_id = graph.loop_end_node_for(&loop_id)?;
            if count == 0 {
                logging::record(
                    workspace,
                    Level::Info,
                    format!("loop node id={loop_id} count=0 (skipping body)"),
                )?;
                let tos = graph.success_targets(&loop_end_id)?;
                return dispatch_successors(
                    graph,
                    bundle,
                    tos,
                    loop_stack,
                    workspace,
                    ws_root,
                    shared_err,
                );
            }
            logging::record(
                workspace,
                Level::Info,
                format!(
                    "loop node id={loop_id} count={count} body_entries={:?} loop_end={loop_end_id}",
                    body_targets
                ),
            )?;
            loop_stack.push(LoopFrame {
                loop_id: loop_id.clone(),
                body_targets: body_targets.clone(),
                loop_end_id,
                count,
                passes_done: 0,
            });
            dispatch_successors(
                graph,
                bundle,
                &body_targets,
                loop_stack,
                workspace,
                ws_root,
                shared_err,
            )
        }
        NodeKind::LoopEnd => {
            let ends = node.ends_loop.as_deref().ok_or_else(|| {
                GraphRunError::msg(format!(
                    "loop_end {:?} missing loop field",
                    node.id
                ))
            })?;
            let frame = loop_stack.last_mut().ok_or_else(|| {
                GraphRunError::msg(format!(
                    "loop_end {:?} reached with no active loop on the stack",
                    node.id
                ))
            })?;
            if frame.loop_id != ends || frame.loop_end_id != node.id {
                return Err(GraphRunError::msg(format!(
                    "loop_end {:?} closes loop {:?}, but the active frame is for loop {:?} / loop_end {:?}",
                    node.id, ends, frame.loop_id, frame.loop_end_id
                )));
            }
            frame.passes_done += 1;
            logging::record(
                workspace,
                Level::Info,
                format!(
                    "loop {} finished body pass {} of {}",
                    frame.loop_id, frame.passes_done, frame.count
                ),
            )?;
            if frame.passes_done < frame.count {
                let body_targets = frame.body_targets.clone();
                dispatch_successors(
                    graph,
                    bundle,
                    &body_targets,
                    loop_stack,
                    workspace,
                    ws_root,
                    shared_err,
                )
            } else {
                loop_stack.pop().expect("non-empty loop stack");
                let tos = graph.success_targets(&node.id)?;
                dispatch_successors(
                    graph,
                    bundle,
                    tos,
                    loop_stack,
                    workspace,
                    ws_root,
                    shared_err,
                )
            }
        }
    }
}

fn loop_env_for_stack(frames: &[LoopFrame]) -> Vec<(String, String)> {
    if frames.is_empty() {
        return Vec::new();
    }
    let f = frames.last().expect("non-empty");
    let body_joined = f.body_targets.join(",");
    vec![
        ("GRAPH_RUN_LOOP_INDEX".into(), f.passes_done.to_string()),
        (
            "GRAPH_RUN_LOOP_ITERATION".into(),
            (f.passes_done + 1).to_string(),
        ),
        ("GRAPH_RUN_LOOP_COUNT".into(), f.count.to_string()),
        ("GRAPH_RUN_LOOP_NODE_ID".into(), f.loop_id.clone()),
        ("GRAPH_RUN_LOOP_BODY_ENTRY".into(), body_joined.clone()),
        ("GRAPH_RUN_LOOP_END_ID".into(), f.loop_end_id.clone()),
        ("GRAPH_RUN_LOOP_BODY_ID".into(), body_joined),
        (
            "GRAPH_RUN_LOOP_DEPTH".into(),
            (frames.len().saturating_sub(1)).to_string(),
        ),
    ]
}

fn execute_task_by_node_id(
    graph: &TaskGraph,
    bundle: &ConfigBundle,
    task_node_id: &str,
    workspace: Option<&Workspace>,
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
