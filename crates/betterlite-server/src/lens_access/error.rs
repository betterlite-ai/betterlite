use std::error::Error as StdError;

use axum::http::StatusCode;
use betterlite_auth::resolved_key::auth::AuthError;
use betterlite_db::lens_repository::repository::LensRepositoryError;
use thiserror::Error;

const WRITE_DENIED: &str = "Only proxy admins can configure or run Lens";
const ACCESS_DENIED: &str = "Lens requires proxy administrator access";
const NO_DATABASE: &str = "Lens needs a connected Postgres database";
const LENS_NOT_FOUND: &str = "Lens not found";
const CONCURRENT_CHANGE: &str = "Lens changed concurrently; retry the operation";
const TRACING_DISABLED: &str =
    "Agent tracing is not enabled. Set `tracing:` in general_settings and CLICKHOUSE_URL.";
const INVALID_SELECTION: &str = "Choose execution IDs returned by the activity preview";
const UNKNOWN_MODEL: &str = "Choose a model configured on this LiteLLM instance";
const MODEL_DENIED: &str = "This key does not have access to the analysis model";
const INVESTIGATION_NOT_FOUND: &str = "Investigation not found";
const WORKER_NOT_FOUND: &str = "Worker not found";
const WORKER_REVOKED: &str = "Register a new worker instead of updating revoked access";
const WORKER_ACCESS_REVOKED: &str = "Worker access was revoked";
const WORKER_RUNNING: &str =
    "Wait for this worker's investigation to finish or cancel it before revoking access";
const EXECUTION_NOT_FOUND: &str = "Execution not found";
const CALLER_UNREADABLE: &str = "the caller key could not be read";
const CALLER_UNBUILT: &str = "the caller key could not be rebuilt";

// every cause in the chain, so a 500 body names what actually failed instead of its first wrapper
pub(crate) fn failure_text(error: &(dyn StdError + 'static)) -> String {
    let mut text = error.to_string();
    let mut current = error.source();
    while let Some(cause) = current {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        current = cause.source();
    }
    text
}

// the refusals the administration handlers and the worker routes share, with LiteLLM's texts
#[derive(Debug, Error)]
pub(crate) enum LensAccessError {
    #[error("{WRITE_DENIED}")]
    WriteDenied,
    #[error("{ACCESS_DENIED}")]
    AccessDenied,
    #[error("{NO_DATABASE}")]
    NoDatabase,
    #[error("{LENS_NOT_FOUND}")]
    LensNotFound,
    #[error("{CONCURRENT_CHANGE}")]
    ConcurrentChange,
    #[error("{TRACING_DISABLED}")]
    TracingDisabled,
    #[error("{INVALID_SELECTION}")]
    InvalidSelection,
    #[error("{UNKNOWN_MODEL}")]
    UnknownModel,
    #[error("{MODEL_DENIED}")]
    ModelDenied,
    #[error("{INVESTIGATION_NOT_FOUND}")]
    InvestigationNotFound,
    #[error("{WORKER_NOT_FOUND}")]
    WorkerNotFound,
    #[error("{WORKER_REVOKED}")]
    WorkerRevoked,
    #[error("{WORKER_ACCESS_REVOKED}")]
    WorkerAccessRevoked,
    #[error("{WORKER_RUNNING}")]
    WorkerRunning,
    #[error("{EXECUTION_NOT_FOUND}")]
    ExecutionNotFound,
    #[error("{CALLER_UNREADABLE}")]
    Caller(#[source] serde_json::Error),
    #[error("{CALLER_UNBUILT}")]
    CallerKey(#[source] AuthError),
    #[error(transparent)]
    Repository(#[from] LensRepositoryError),
}

impl LensAccessError {
    #[must_use]
    pub(crate) const fn status(&self) -> StatusCode {
        match self {
            Self::WriteDenied | Self::AccessDenied | Self::ModelDenied => StatusCode::FORBIDDEN,
            Self::NoDatabase => StatusCode::SERVICE_UNAVAILABLE,
            Self::LensNotFound
            | Self::InvestigationNotFound
            | Self::WorkerNotFound
            | Self::ExecutionNotFound => StatusCode::NOT_FOUND,
            Self::ConcurrentChange
            | Self::WorkerRevoked
            | Self::WorkerAccessRevoked
            | Self::WorkerRunning => StatusCode::CONFLICT,
            Self::TracingDisabled => StatusCode::NOT_IMPLEMENTED,
            Self::InvalidSelection => StatusCode::UNPROCESSABLE_ENTITY,
            Self::UnknownModel => StatusCode::BAD_REQUEST,
            Self::Caller(_) | Self::CallerKey(_) | Self::Repository(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    #[must_use]
    pub(crate) fn detail(&self) -> String {
        match self {
            Self::Caller(source) => format!("{CALLER_UNREADABLE}: {}", failure_text(source)),
            Self::CallerKey(source) => format!("{CALLER_UNBUILT}: {}", failure_text(source)),
            Self::Repository(source) => failure_text(source),
            Self::WriteDenied => WRITE_DENIED.to_owned(),
            Self::AccessDenied => ACCESS_DENIED.to_owned(),
            Self::NoDatabase => NO_DATABASE.to_owned(),
            Self::LensNotFound => LENS_NOT_FOUND.to_owned(),
            Self::ConcurrentChange => CONCURRENT_CHANGE.to_owned(),
            Self::TracingDisabled => TRACING_DISABLED.to_owned(),
            Self::InvalidSelection => INVALID_SELECTION.to_owned(),
            Self::UnknownModel => UNKNOWN_MODEL.to_owned(),
            Self::ModelDenied => MODEL_DENIED.to_owned(),
            Self::InvestigationNotFound => INVESTIGATION_NOT_FOUND.to_owned(),
            Self::WorkerNotFound => WORKER_NOT_FOUND.to_owned(),
            Self::WorkerRevoked => WORKER_REVOKED.to_owned(),
            Self::WorkerAccessRevoked => WORKER_ACCESS_REVOKED.to_owned(),
            Self::WorkerRunning => WORKER_RUNNING.to_owned(),
            Self::ExecutionNotFound => EXECUTION_NOT_FOUND.to_owned(),
        }
    }
}

mod tests;
