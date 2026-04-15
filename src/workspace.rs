//! Workspace root: `logs/` for run logs, `tmp/` for scratch files.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{GraphRunError, Result};

fn is_read_only_fs_error(err: &std::io::Error) -> bool {
    matches!(err.kind(), ErrorKind::ReadOnlyFilesystem)
        || err.raw_os_error() == Some(30) // EROFS on many Unix targets
}

fn map_workspace_dir_err(path: PathBuf, source: std::io::Error) -> GraphRunError {
    if is_read_only_fs_error(&source) {
        GraphRunError::msg(format!(
            "cannot create workspace directory {}: {}.\n\
This path is on a read-only filesystem. On macOS, do not use `--workspace /config`: `/config` exists only inside the SSH test *container* (bind-mounted from the host). Run `graph_run` on the host with the default `--workspace .workspace` (or any writable directory under your project), not a system path.",
            path.display(),
            source
        ))
    } else {
        GraphRunError::Io { file: path, source }
    }
}

pub struct Workspace {
    root: PathBuf,
    tmp: PathBuf,
    log_path: PathBuf,
    log: Mutex<std::fs::File>,
}

impl Workspace {
    /// Creates `root`, `root/logs`, and `root/tmp`, and opens a new log file under `logs/`.
    pub fn prepare(root: PathBuf) -> Result<Self> {
        let mut root = root;
        let cwd_for_log = std::env::current_dir().ok();
        if root.is_relative() {
            root = std::env::current_dir()
                .map_err(|e| {
                    GraphRunError::msg(format!(
                        "cannot resolve workspace path (current directory unavailable): {e}"
                    ))
                })?
                .join(root);
        }
        log::debug!(
            target: "graph_run",
            "workspace: cwd={:?} resolved_root={}",
            cwd_for_log.as_ref().map(|p| p.display().to_string()),
            root.display(),
        );

        fs::create_dir_all(&root)
            .map_err(|source| map_workspace_dir_err(root.clone(), source))?;
        let logs = root.join("logs");
        let tmp = root.join("tmp");
        fs::create_dir_all(&logs)
            .map_err(|source| map_workspace_dir_err(logs.clone(), source))?;
        fs::create_dir_all(&tmp)
            .map_err(|source| map_workspace_dir_err(tmp.clone(), source))?;

        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let log_path = logs.join(format!("run_{stamp}.log"));
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|source| GraphRunError::Io {
                file: log_path.clone(),
                source,
            })?;

        Ok(Self {
            root,
            tmp,
            log_path,
            log: Mutex::new(log),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn tmp_dir(&self) -> &Path {
        &self.tmp
    }

    pub fn log_file_path(&self) -> &Path {
        &self.log_path
    }

    pub fn log_line(&self, line: &str) -> Result<()> {
        let mut f = self
            .log
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        writeln!(f, "{line}").map_err(|source| GraphRunError::Io {
            file: self.log_path.clone(),
            source,
        })?;
        f.flush().map_err(|source| GraphRunError::Io {
            file: self.log_path.clone(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::Workspace;

    /// Serialize tests that `chdir` so the process cwd is restored reliably.
    static CWD_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn prepare_absolute_root_creates_logs_tmp_and_log_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("ws_abs");
        let ws = Workspace::prepare(root.clone()).expect("prepare");
        assert_eq!(ws.root(), root);
        assert_eq!(ws.tmp_dir(), root.join("tmp"));
        assert!(ws.log_file_path().starts_with(root.join("logs")));
        assert!(root.join("logs").is_dir());
        assert!(root.join("tmp").is_dir());
        ws.log_line("line-a").expect("log");
        ws.log_line("line-b").expect("log");
        let body = fs::read_to_string(ws.log_file_path()).expect("read log");
        assert!(body.contains("line-a"));
        assert!(body.contains("line-b"));
    }

    #[test]
    fn prepare_resolves_relative_root_against_cwd() {
        let _lock = CWD_TEST_LOCK.lock().expect("cwd test lock");
        let tmp = tempfile::tempdir().expect("tempdir");
        let old = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(tmp.path()).expect("chdir temp");
        let ws = Workspace::prepare(PathBuf::from("rel_graph_run_ws")).expect("prepare");
        let expected = tmp.path().join("rel_graph_run_ws");
        assert_eq!(
            ws.root().canonicalize().expect("canon root"),
            expected.canonicalize().expect("canon expected")
        );
        assert_eq!(ws.tmp_dir(), ws.root().join("tmp"));
        ws.log_line("from-relative").expect("log");
        let body = fs::read_to_string(ws.log_file_path()).expect("read log");
        assert!(body.contains("from-relative"));
        std::env::set_current_dir(&old).expect("restore cwd");
    }
}
