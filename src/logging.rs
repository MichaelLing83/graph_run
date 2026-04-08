//! `log` facade: stderr via `env_logger`, optional duplicate lines to workspace files.

use log::{Level, LevelFilter, log_enabled};

use crate::error::Result;
use crate::workspace::Workspace;

pub const TARGET: &str = "graph_run";

/// Initialize `env_logger`. If `RUST_LOG` is set, it controls filters; otherwise `--verbose` count
/// sets the level for the `graph_run` target only (other crates stay quiet).
pub fn init(verbose_count: u8) {
    use env_logger::Env;
    if std::env::var_os("RUST_LOG").is_some() {
        let _ = env_logger::Builder::from_env(Env::default()).try_init();
        return;
    }
    let level = level_filter_from_verbose(verbose_count);
    let _ = env_logger::Builder::new()
        .filter_level(LevelFilter::Off)
        .filter_module(TARGET, level)
        .format_timestamp_secs()
        .try_init();
}

fn level_filter_from_verbose(v: u8) -> LevelFilter {
    match v {
        0 => LevelFilter::Error,
        1 => LevelFilter::Warn,
        2 => LevelFilter::Info,
        3 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    }
}

/// Emit to the global logger (stderr) and, if `workspace` is set and this level is enabled, append
/// a line to the workspace run log.
pub fn record(
    workspace: &mut Option<&mut Workspace>,
    level: Level,
    msg: impl AsRef<str>,
) -> Result<()> {
    let msg = msg.as_ref();
    log::log!(target: TARGET, level, "{msg}");
    if log_enabled!(target: TARGET, level) {
        if let Some(w) = workspace.as_deref_mut() {
            w.log_line(&format!("[{}] {msg}", level.as_str()))?;
        }
    }
    Ok(())
}
