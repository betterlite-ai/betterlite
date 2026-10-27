use std::io;
use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;
use tokio::task::JoinError;

use crate::errors::HarnessError;

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("sandbox is closed")]
    Closed,
    #[error("sandbox state poisoned")]
    Poisoned,
    #[error("sandbox path does not exist or is not a directory: {}: {source}", path.display())]
    Root { path: PathBuf, source: io::Error },
    #[error("path escapes the sandbox: {}", .0.display())]
    Escape(PathBuf),
    #[error("path is not UTF-8: {}", .0.display())]
    PathEncoding(PathBuf),
    #[error("exec() needs a non-empty command")]
    EmptyCommand,
    #[error("cwd is not a directory: {}", .0.display())]
    Cwd(PathBuf),
    #[error("could not start {name}: {source}")]
    Start { name: String, source: io::Error },
    #[error("subprocess was started without stdout/stderr pipes")]
    Pipes,
    #[error("command timed out after {timeout:?}: {name}")]
    Timeout { timeout: Duration, name: String },
    #[error("sandbox worker failed: {source}")]
    Worker { source: JoinError },
    #[error("sandbox worker ended without a result")]
    NoResult,
}

impl From<SandboxError> for HarnessError {
    fn from(source: SandboxError) -> Self {
        Self::Sandbox {
            source: io::Error::other(source),
        }
    }
}
