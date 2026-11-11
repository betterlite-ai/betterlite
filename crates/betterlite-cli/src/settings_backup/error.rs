use std::io;
use std::path::PathBuf;

use betterlite_logging::private_json::error::PrivateJsonError;
use thiserror::Error;

use crate::claude_settings::error::ClaudeSettingsError;

#[derive(Debug, Error)]
pub enum BackupError {
    #[error("the environment has no home directory")]
    NoHome,
    #[error("cannot prepare the private backup directory: {source}")]
    Directory {
        #[source]
        source: PrivateJsonError,
    },
    #[error("cannot create {path}: {source}")]
    Create {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot set the owner-only mode on {path}: {source}")]
    Mode {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot write the backup {path}: {source}")]
    WriteBackup {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot encode the backup for {path}: {source}")]
    Encode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("cannot read the backup {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{path} contains invalid or unexpected JSON; cannot restore from it safely.")]
    Invalid {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("cannot create the directory for {path}: {source}")]
    SettingsDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot write {path}: {source}")]
    WriteSettings {
        path: PathBuf,
        #[source]
        source: ClaudeSettingsError,
    },
    #[error("cannot remove {path}: {source}")]
    RemoveSettings {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot remove the backup {path}: {source}")]
    RemoveBackup {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the async runtime could not start: {source}")]
    Runtime {
        #[source]
        source: io::Error,
    },
}
