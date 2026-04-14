use std::path::PathBuf;

use clap::ArgAction;
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
    /// More verbose logging on stderr (and workspace log when enabled). Repeat for higher levels:
    /// error (default) → warn (-v) → info (-vv) → debug (-vvv) → trace (-vvvv+).
    /// If `RUST_LOG` is set, it overrides this for `env_logger`.
    #[arg(short, long, action = ArgAction::Count)]
    verbose: u8,

    /// Directory for run logs (`logs/`) and scratch files (`tmp/`). Default: `.workspace` under the
    /// process current working directory.
    #[arg(
        long,
        value_name = "DIR",
        default_value = ".workspace",
        value_parser = parse_config_path
    )]
    workspace: PathBuf,

    /// Allow workflows whose success-edge graph contains a directed cycle (can run forever if
    /// every task succeeds). Without this flag, such workflows are rejected.
    #[arg(long)]
    allow_endless_loop: bool,

    /// Optional TOML file of scalar constants; `${NAME}` in each config file is replaced before
    /// parsing (not applied to the constants file itself).
    #[arg(long, value_name = "FILE", value_parser = parse_config_path)]
    constants: Option<PathBuf>,

    /// TOML config file(s): each may define any of `servers`, `shells`, `commands`, `tasks`,
    /// `nodes`, `edges` (see README). Multiple paths are merged in order; later rows append after
    /// earlier ones per section.
    #[arg(
        value_name = "FILE",
        value_parser = parse_config_path,
        num_args = 1..,
        required = true
    )]
    configs: Vec<PathBuf>,
}

fn main() {
    let cli = Cli::parse();
    graph_run::logging::init(cli.verbose);
    if let Err(e) = graph_run::run_with_configs(
        &cli.configs,
        Some(cli.workspace.as_path()),
        cli.allow_endless_loop,
        cli.constants.as_deref(),
    ) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
