use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum GraphRunError {
    #[error("I/O error reading {file}: {source}")]
    Io {
        file: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse TOML in {file}: {source}")]
    Toml {
        file: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("{0}")]
    Msg(String),
}

impl GraphRunError {
    pub fn msg(s: impl Into<String>) -> Self {
        Self::Msg(s.into())
    }
}

pub type Result<T> = std::result::Result<T, GraphRunError>;
