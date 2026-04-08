//! Workspace root: `logs/` for run logs, `tmp/` for scratch files.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{GraphRunError, Result};

pub struct Workspace {
    root: PathBuf,
    tmp: PathBuf,
    log_path: PathBuf,
    log: std::fs::File,
}

impl Workspace {
    /// Creates `root`, `root/logs`, and `root/tmp`, and opens a new log file under `logs/`.
    pub fn prepare(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root).map_err(|source| GraphRunError::Io {
            file: root.clone(),
            source,
        })?;
        let logs = root.join("logs");
        let tmp = root.join("tmp");
        fs::create_dir_all(&logs).map_err(|source| GraphRunError::Io {
            file: logs.clone(),
            source,
        })?;
        fs::create_dir_all(&tmp).map_err(|source| GraphRunError::Io {
            file: tmp.clone(),
            source,
        })?;

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
            log,
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

    pub fn log_line(&mut self, line: &str) -> Result<()> {
        writeln!(self.log, "{line}").map_err(|source| GraphRunError::Io {
            file: self.log_path.clone(),
            source,
        })?;
        self.log.flush().map_err(|source| GraphRunError::Io {
            file: self.log_path.clone(),
            source,
        })
    }
}
