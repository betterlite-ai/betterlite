use std::io;
use std::path::PathBuf;

use betterlite_logging::private_json::error::PrivateJsonError;
use thiserror::Error;

use crate::claude_settings::error::ClaudeSettingsError;

#[derive(Debug, Error)]
pub enum ReceiptsError {
    #[error(transparent)]
    Settings(#[from] ClaudeSettingsError),
    #[error("cannot read the configure receipt {path}: {source}")]
    ReadReceipt {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "{path} is not a readable `lite configure claude` receipt: {source}; remove it and edit Claude Code's settings by hand if they still point at the proxy"
    )]
    InvalidReceipt {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "Claude Code is not configured by `lite configure claude` (no receipt at {path}); nothing to undo"
    )]
    NoReceipt { path: PathBuf },
    #[error("cannot encode the configure receipt for {path}: {source}")]
    EncodeReceipt {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("cannot write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: PrivateJsonError,
    },
    #[error("cannot remove {path}: {source}")]
    Remove {
        path: PathBuf,
        #[source]
        source: PrivateJsonError,
    },
    #[error("cannot discard the staged file {path}: {source}")]
    Discard {
        path: PathBuf,
        #[source]
        source: PrivateJsonError,
    },
    #[error(
        "{settings} The receipt at {state_path} now describes settings that were not written and could not be put back either ({receipt}); remove it before retrying"
    )]
    Inconsistent {
        state_path: PathBuf,
        #[source]
        settings: Box<Self>,
        receipt: Box<Self>,
    },
}
