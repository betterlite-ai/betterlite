use std::io;
use std::num::TryFromIntError;

use thiserror::Error;
use tokio::task::JoinError;

use super::body::BodyError;
use super::spool::SpoolError;
use crate::api::file_batch_guardrails::error::BatchGuardrailError;

// What can end a scan besides a decided file: an unreadable line, a blocking read the runtime cancelled, a spool that could not take the rewrites, or a source that would not rewind
#[derive(Debug, Error)]
pub enum ScanFileError {
    #[error(transparent)]
    Input(#[from] BatchGuardrailError),
    #[error(transparent)]
    Spool(#[from] SpoolError),
    #[error("the batch input file's blocking read task failed")]
    Task {
        #[source]
        source: JoinError,
    },
    #[error("rewinding the batch input file failed")]
    Rewind {
        #[source]
        source: io::Error,
    },
    #[error("a rewritten record is larger than this platform addresses")]
    Length {
        #[source]
        source: TryFromIntError,
    },
}

impl From<BodyError> for ScanFileError {
    fn from(error: BodyError) -> Self {
        match error {
            BodyError::Input(source) => Self::Input(source),
            BodyError::Task { source } => Self::Task { source },
        }
    }
}

impl From<io::Error> for ScanFileError {
    fn from(source: io::Error) -> Self {
        Self::Rewind { source }
    }
}

// What can end a rewrite: the same unreadable line, cancelled blocking read, spool and rewind failures
#[derive(Debug, Error)]
pub enum RewriteError {
    #[error(transparent)]
    Input(#[from] BatchGuardrailError),
    #[error(transparent)]
    Spool(#[from] SpoolError),
    #[error("the batch input file's blocking read task failed")]
    Task {
        #[source]
        source: JoinError,
    },
    #[error("rewinding the batch input file failed")]
    Rewind {
        #[source]
        source: io::Error,
    },
}

impl From<BodyError> for RewriteError {
    fn from(error: BodyError) -> Self {
        match error {
            BodyError::Input(source) => Self::Input(source),
            BodyError::Task { source } => Self::Task { source },
        }
    }
}

impl From<io::Error> for RewriteError {
    fn from(source: io::Error) -> Self {
        Self::Rewind { source }
    }
}
