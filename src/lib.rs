//! Load TOML configs (servers, shells, commands, tasks, workflow), build a task graph, and run it.

mod config;
mod env_merge;
mod error;
mod execute;
mod workflow;
mod workspace;

pub use error::GraphRunError;
pub use error::Result;
pub use workspace::Workspace;

use std::path::Path;

/// Load all configuration files and execute the workflow graph.
///
/// If `workspace` is set, creates `logs/` and `tmp/` under that directory, writes a per-run log
/// file, and sets `GRAPH_RUN_WORKSPACE` / `GRAPH_RUN_TMP` in the environment for local tasks.
pub fn run_with_paths(
    servers: &Path,
    shells: &Path,
    commands: &Path,
    tasks: &Path,
    workflow: &Path,
    workspace: Option<&Path>,
) -> Result<()> {
    let bundle = config::load_bundle(servers, shells, commands, tasks, workflow)?;
    if let Some(dir) = workspace {
        let mut ws = Workspace::prepare(dir.to_path_buf())?;
        workflow::run_workflow(&bundle, Some(&mut ws))
    } else {
        workflow::run_workflow(&bundle, None)
    }
}
