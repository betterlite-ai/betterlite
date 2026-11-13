use std::io;
use std::path::PathBuf;

use tokio::sync::AcquireError;
use tokio::task::JoinError;

#[derive(Debug, thiserror::Error)]
pub enum PrivateJsonError {
    #[error("the private JSON path {0} names no file or directory")]
    InvalidPath(PathBuf),
    #[error("cannot prepare the private directory {0}: {1}")]
    Directory(PathBuf, #[source] io::Error),
    #[error("cannot read the mode of the private directory {0}: {1}")]
    DirectoryMode(PathBuf, #[source] io::Error),
    #[error("cannot tighten the private directory {0} to owner-only: {1}")]
    DirectoryTighten(PathBuf, #[source] io::Error),
    #[error("cannot stage a private file for {0}: {1}")]
    Stage(PathBuf, #[source] io::Error),
    #[error("cannot commit a staged private file into {0}: {1}")]
    Commit(PathBuf, #[source] io::Error),
    #[error("cannot discard the staged private file {0}: {1}")]
    Discard(PathBuf, #[source] io::Error),
    #[error("cannot rewrite the existing private file {0}: {1}")]
    Overwrite(PathBuf, #[source] io::Error),
    #[error("the JSON payload cannot be encoded: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("the JSON payload cannot be decoded: {0}")]
    Decode(#[source] serde_json::Error),
    #[error("the JSON payload is {0} bytes, over the {1}-byte limit")]
    TooLarge(usize, usize),
    #[error("the JSON payload nests deeper than {0} levels")]
    TooDeep(usize),
    #[error("the private file worker pool is unavailable: {0}")]
    WorkerUnavailable(#[source] AcquireError),
    #[error("the private file worker did not finish: {0}")]
    Worker(#[source] JoinError),
    #[error("the private file worker stopped without running")]
    WorkerStopped,
}
