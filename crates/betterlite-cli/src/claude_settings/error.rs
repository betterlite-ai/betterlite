use std::io;
use std::path::PathBuf;

use betterlite_logging::private_json::error::PrivateJsonError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClaudeSettingsError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{path} does not hold a JSON object: {source}")]
    NotObject {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("cannot resolve {path}: {source}")]
    Resolve {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: PrivateJsonError,
    },
    #[error(
        "{path} has a non-object \"env\" value, which betterlite would discard; fix or remove it, then retry"
    )]
    NonObjectEnv { path: PathBuf },
    #[error(
        "`{start_command}` is managing {path} (backup at {backup_path}) and restores it when it stops; run `{stop_command}` first, then retry"
    )]
    Owned {
        path: PathBuf,
        backup_path: PathBuf,
        start_command: &'static str,
        stop_command: &'static str,
    },
    #[error("cannot locate the betterlite executable: {source}")]
    CurrentExe {
        #[source]
        source: io::Error,
    },
    #[error("cannot quote {path} for the command line: {source}")]
    Quote {
        path: PathBuf,
        #[source]
        source: shlex::QuoteError,
    },
    #[error("{path} is not valid UTF-8, so it cannot be quoted for the command line")]
    NonUtf8Path { path: PathBuf },
}

mod tests;
