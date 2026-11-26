use std::error::Error;

use betterlite_auth::resolved_key::auth::AuthError;
use betterlite_models::decisions::Json;
use thiserror::Error;

use crate::api::api_boundaries::ApiLimitError;

// FastAPI's HTTPException as this slice raises and re-raises it: a status and a JSON detail
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchHttpError {
    pub status: u16,
    pub detail: Json,
}

// The ValueErrors _fetch_managed_file_content raises before it calls the hook
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BatchManagedFileError {
    #[error("proxy_logging_obj not available. Cannot access managed files hook.")]
    LoggingMissing,
    #[error("Managed files hook not found. Cannot access managed file.")]
    HookMissing,
    #[error("Managed files hook is not a BaseFileEndpoints instance.")]
    NotFileEndpoints,
    #[error("llm_router not available. Cannot access managed files.")]
    RouterMissing,
}

// Every failure of one batch input-accounting call; only Http keeps the 403 that reaches the caller
#[derive(Debug, Error)]
pub enum BatchInputAccountingError<CallsError, FilesError, AccessError>
where
    CallsError: Error + 'static,
    FilesError: Error + 'static,
    AccessError: Error + 'static,
{
    #[error(transparent)]
    Principal(#[from] AuthError),
    #[error("failed to read the batch caller fields")]
    Fields(#[source] serde_json::Error),
    #[error(transparent)]
    Boundary(#[from] ApiLimitError),
    #[error(transparent)]
    Calls(CallsError),
    #[error(transparent)]
    Files(FilesError),
    #[error(transparent)]
    Access(AccessError),
    #[error("the batch file request was rejected over HTTP")]
    Http(BatchHttpError),
    #[error(transparent)]
    Managed(#[from] BatchManagedFileError),
    #[error("Expected bytes content from file retrieval for {file_id}, got {kind}")]
    ContentNotBytes { file_id: String, kind: String },
}

impl<CallsError, FilesError, AccessError>
    BatchInputAccountingError<CallsError, FilesError, AccessError>
where
    CallsError: Error + 'static,
    FilesError: Error + 'static,
    AccessError: Error + 'static,
{
    // The HTTPException behind this failure, when the slice re-raises one as FastAPI does
    #[must_use]
    pub const fn http(&self) -> Option<&BatchHttpError> {
        match self {
            Self::Http(http) => Some(http),
            Self::Principal(_)
            | Self::Fields(_)
            | Self::Boundary(_)
            | Self::Calls(_)
            | Self::Files(_)
            | Self::Access(_)
            | Self::Managed(_)
            | Self::ContentNotBytes { .. } => None,
        }
    }
}
