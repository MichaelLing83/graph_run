use std::path::PathBuf;

use clap::Parser;

fn parse_config_path(s: &str) -> Result<PathBuf, String> {
    let p = s.strip_prefix('@').unwrap_or(s);
    if p.is_empty() {
        return Err("path is empty".into());
    }
    Ok(PathBuf::from(p))
}

#[derive(Parser, Debug)]
#[command(name = "graph_run", version, about = "Run a task graph from TOML configuration")]
struct Cli {
    /// Servers inventory (00_servers.toml)
    #[arg(long, visible_alias = "server", value_name = "FILE", value_parser = parse_config_path)]
    servers: PathBuf,

    /// Shell profiles (01_shells.toml)
    #[arg(long, value_name = "FILE", value_parser = parse_config_path)]
    shells: PathBuf,

    /// Command definitions (02_commands.toml)
    #[arg(long, value_name = "FILE", value_parser = parse_config_path)]
    commands: PathBuf,

    /// Task bindings server + shell + command (03_tasks.toml)
    #[arg(long, value_name = "FILE", value_parser = parse_config_path)]
    tasks: PathBuf,

    /// Directory for run logs (`logs/`) and scratch files (`tmp/`). Optional.
    #[arg(long, value_name = "DIR", value_parser = parse_config_path)]
    workspace: Option<PathBuf>,

    /// Workflow graph: nodes + edges (04_workflow.toml)
    #[arg(value_name = "WORKFLOW", value_parser = parse_config_path)]
    workflow: PathBuf,
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = graph_run::run_with_paths(
        &cli.servers,
        &cli.shells,
        &cli.commands,
        &cli.tasks,
        &cli.workflow,
        cli.workspace.as_deref(),
    ) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
