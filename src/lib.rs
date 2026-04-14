//! Load merged TOML config (servers, shells, commands, tasks, workflow), build a task graph, and run it.

mod config;
mod config_merge;
mod constants;
mod env_merge;
mod error;
mod execute;
pub mod logging;
mod transfer;
mod workflow;
mod workflow_viz;
mod workspace;

pub use error::GraphRunError;
pub use error::Result;
pub use workflow_viz::WorkflowVizFormat;
pub use workspace::Workspace;

use std::path::Path;

/// Load merged configuration from one or more TOML paths and execute the workflow graph.
///
/// If `workspace` is `Some`, creates `logs/` and `tmp/` under that directory, writes a per-run log
/// file, and sets `GRAPH_RUN_WORKSPACE` / `GRAPH_RUN_TMP` in the environment for local tasks. The
/// CLI uses **`.workspace`** in the current directory when `--workspace` is omitted; pass `None`
/// only when embedding the library without a workspace directory.
///
/// Unless `allow_endless_loop` is true, workflows with a directed cycle on **success** edges are
/// rejected (they could run forever while every task succeeds).
///
/// Multiple `[[edges]]` rows with the same `from` define **parallel** fan-out; branches that meet
/// at a node with more than one incoming success edge **join** there (barrier) before that node runs.
///
/// Workflow files may omit `[[nodes]]` for **`start`**, **`end`**, and **`abort`**; those nodes are
/// added with the expected kinds when missing (`config::WorkflowFile::ensure_default_control_nodes`).
///
/// Each `[[edges]]` row has a **`failure`** target defaulting to **`abort`** when omitted
/// (`config::WorkflowEdge`). `[[tasks]]` may set **`retry`** (default `0`) to re-run a failed task
/// (command or transfer) up to that many extra times before taking the failure transition.
///
/// If `constants` is set, that TOML file is loaded first; every **`${IDENT}`** placeholder in each
/// config file is replaced with the matching scalar value before parsing (see README).
pub fn run_with_configs<P: AsRef<Path>>(
    config_files: &[P],
    workspace: Option<&Path>,
    allow_endless_loop: bool,
    constants: Option<&Path>,
) -> Result<()> {
    let bundle = config::load_bundle(config_files, constants)?;
    if let Some(dir) = workspace {
        let mut ws = Workspace::prepare(dir.to_path_buf())?;
        workflow::run_workflow(&bundle, Some(&mut ws), allow_endless_loop)
    } else {
        workflow::run_workflow(&bundle, None, allow_endless_loop)
    }
}

/// Load merged configuration and render the workflow graph in a textual format.
pub fn visualize_with_configs<P: AsRef<Path>>(
    config_files: &[P],
    constants: Option<&Path>,
    format: WorkflowVizFormat,
) -> Result<String> {
    let bundle = config::load_bundle(config_files, constants)?;
    Ok(workflow_viz::render(&bundle.workflow, format))
}

/// Load merged configuration and serialize it into a normalized single TOML document.
pub fn merge_with_configs<P: AsRef<Path>>(
    config_files: &[P],
    constants: Option<&Path>,
) -> Result<String> {
    let bundle = config::load_bundle(config_files, constants)?;
    Ok(config_merge::merge_bundle_to_toml(&bundle))
}
