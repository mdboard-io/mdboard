use std::path::PathBuf;

use thiserror::Error;

/// Errors produced while loading or validating a board.
#[derive(Debug, Error)]
pub enum Error {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("card error in {path}: {message}")]
    Card { path: PathBuf, message: String },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("TOML error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("{0}")]
    Message(String),
}

impl Error {
    #[must_use]
    pub fn card(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::Card {
            path: path.into(),
            message: message.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
