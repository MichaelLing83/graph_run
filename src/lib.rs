//! Load merged TOML config (servers, shells, commands, tasks, workflow), build a task graph, and run it.

mod config;
mod constants;
mod env_merge;
mod error;
mod execute;
pub mod logging;
mod transfer;
mod workflow;
mod workspace;

pub use error::GraphRunError;
pub use error::Result;
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
/// (`config::WorkflowEdge`).
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
