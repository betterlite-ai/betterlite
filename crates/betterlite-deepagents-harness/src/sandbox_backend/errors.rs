use betterlite_harness::errors::HarnessError;
use thiserror::Error;

use crate::sandbox_backend::include_glob::InvalidGlobPattern;

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("path not found: {path}")]
    NotFound { path: String },
    #[error("path resolves outside the workspace: {path}")]
    OutsideWorkspace { path: String },
    #[error(transparent)]
    Sandbox(#[from] HarnessError),
    #[error(transparent)]
    Glob(#[from] InvalidGlobPattern),
    #[error(transparent)]
    GrepLine(#[from] regex::Error),
    #[error("SandboxBackend sync methods cannot run on the event loop thread")]
    EventLoopThread,
}

#[derive(Debug, Error)]
pub enum TransferError {
    #[error("file_not_found")]
    FileNotFound,
    #[error("permission_denied")]
    PermissionDenied,
    #[error("invalid_path")]
    InvalidPath,
    #[error("{0}")]
    Sandbox(#[source] HarnessError),
}
