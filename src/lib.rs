//! Load TOML configs (servers, shells, commands, tasks, workflow), build a task graph, and run it.

mod config;
mod env_merge;
mod error;
mod execute;
mod workflow;

pub use error::GraphRunError;
pub use error::Result;

use std::path::Path;

/// Load all configuration files and execute the workflow graph.
pub fn run_with_paths(
    servers: &Path,
    shells: &Path,
    commands: &Path,
    tasks: &Path,
    workflow: &Path,
) -> Result<()> {
    let bundle = config::load_bundle(servers, shells, commands, tasks, workflow)?;
    workflow::run_workflow(&bundle)
}
