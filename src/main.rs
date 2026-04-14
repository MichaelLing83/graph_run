use std::path::PathBuf;

use clap::ArgAction;
use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;

fn parse_config_path(s: &str) -> Result<PathBuf, String> {
    let p = s.strip_prefix('@').unwrap_or(s);
    if p.is_empty() {
        return Err("path is empty".into());
    }
    Ok(PathBuf::from(p))
}

#[derive(Parser, Debug)]
#[command(
    name = "graph_run",
    version,
    about = "Run a task graph from TOML configuration",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

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
        num_args = 1..
    )]
    configs: Vec<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Render the merged workflow graph from TOML configs.
    Visualize(VisualizeCli),
    /// Merge input TOML configs into one normalized TOML file.
    Merge(MergeCli),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum VisualizeFormat {
    Mermaid,
    Ascii,
}

#[derive(Parser, Debug)]
#[command(
    name = "graph_run visualize",
    about = "Render the merged workflow graph from TOML configs"
)]
struct VisualizeCli {
    /// More verbose diagnostics on stderr.
    #[arg(short, long, action = ArgAction::Count)]
    verbose: u8,

    /// Optional TOML file of scalar constants; `${NAME}` in each config file is replaced before
    /// parsing (not applied to the constants file itself).
    #[arg(long, value_name = "FILE", value_parser = parse_config_path)]
    constants: Option<PathBuf>,

    /// Output format for workflow graph rendering.
    #[arg(short = 'F', long, value_enum, default_value_t = VisualizeFormat::Mermaid)]
    format: VisualizeFormat,

    /// Write output to this file path instead of stdout.
    #[arg(short, long, value_name = "FILE", value_parser = parse_config_path)]
    output: Option<PathBuf>,

    /// TOML config file(s): each may define any of `servers`, `shells`, `commands`, `tasks`,
    /// `nodes`, `edges` (see README). Multiple paths are merged in order; later rows append after
    /// earlier rows per section.
    #[arg(
        value_name = "FILE",
        value_parser = parse_config_path,
        num_args = 1..,
        required = true
    )]
    configs: Vec<PathBuf>,
}

#[derive(Parser, Debug)]
#[command(
    name = "graph_run merge",
    about = "Merge input TOML configs into one normalized TOML file"
)]
struct MergeCli {
    /// More verbose diagnostics on stderr.
    #[arg(short, long, action = ArgAction::Count)]
    verbose: u8,

    /// Optional TOML file of scalar constants; `${NAME}` in each config file is replaced before
    /// parsing (not applied to the constants file itself).
    #[arg(long, value_name = "FILE", value_parser = parse_config_path)]
    constants: Option<PathBuf>,

    /// Write output to this file path instead of stdout.
    #[arg(short, long, value_name = "FILE", value_parser = parse_config_path)]
    output: Option<PathBuf>,

    /// TOML config file(s): each may define any of `servers`, `shells`, `commands`, `tasks`,
    /// `nodes`, `edges` (see README). Multiple paths are merged in order; later rows append after
    /// earlier rows per section.
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
    match cli.command {
        Some(Commands::Visualize(cli)) => {
            graph_run::logging::init(cli.verbose);
            let format = match cli.format {
                VisualizeFormat::Mermaid => graph_run::WorkflowVizFormat::Mermaid,
                VisualizeFormat::Ascii => graph_run::WorkflowVizFormat::Ascii,
            };
            match graph_run::visualize_with_configs(&cli.configs, cli.constants.as_deref(), format)
            {
                Ok(rendered) => {
                    if let Some(path) = cli.output {
                        if let Err(e) = std::fs::write(&path, rendered) {
                            eprintln!("failed to write {}: {e}", path.display());
                            std::process::exit(1);
                        }
                    } else {
                        println!("{rendered}");
                    }
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::Merge(cli)) => {
            graph_run::logging::init(cli.verbose);
            match graph_run::merge_with_configs(&cli.configs, cli.constants.as_deref()) {
                Ok(rendered) => {
                    if let Some(path) = cli.output {
                        if let Err(e) = std::fs::write(&path, rendered) {
                            eprintln!("failed to write {}: {e}", path.display());
                            std::process::exit(1);
                        }
                    } else {
                        println!("{rendered}");
                    }
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        None => {
            if cli.configs.is_empty() {
                eprintln!("at least one config file is required");
                std::process::exit(2);
            }
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
    }
}
