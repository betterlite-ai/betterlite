use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::postgres_store::errors::DbError;
use thiserror::Error;

use crate::benchmark_aggregates::decode::DecodeError;
use crate::benchmark_aggregates::window::WindowError;
use crate::shadow_eval_lifecycle::error::DB_NOT_CONNECTED;
use crate::shadow_eval_validation::error::AccessError;

use super::query::ReadQueryError;

// the benchmark and session reads' failures, each with the status LiteLLM answers
#[derive(Debug, Error)]
pub enum ReadApiError {
    #[error(transparent)]
    Access(#[from] AccessError),
    #[error(transparent)]
    Query(#[from] ReadQueryError),
    #[error(transparent)]
    Window(#[from] WindowError),
    #[error("{}", DB_NOT_CONNECTED)]
    DatabaseUnavailable,
    // the auth layer published a principal this server could not read back
    #[error("the caller's identity could not be read: {source}")]
    Identity {
        #[source]
        source: serde_json::Error,
    },
    #[error("the caller's key could not be read: {source}")]
    CallerKey {
        #[source]
        source: serde_json::Error,
    },
    #[error("the auto-router rollup read failed: {message}")]
    Store { message: String },
    #[error("No auto-routed turns recorded for session {session} under this key")]
    SessionNotFound { session: String },
}

impl ReadApiError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Access(error) => error.status_code(),
            Self::Window(_) => 400,
            Self::Query(_) => 422,
            Self::SessionNotFound { .. } => 404,
            Self::DatabaseUnavailable
            | Self::Identity { .. }
            | Self::CallerKey { .. }
            | Self::Store { .. } => 500,
        }
    }
}

impl From<DbError> for ReadApiError {
    fn from(source: DbError) -> Self {
        Self::Store {
            message: source.to_string(),
        }
    }
}

impl From<TableError> for ReadApiError {
    fn from(source: TableError) -> Self {
        Self::Store {
            message: source.to_string(),
        }
    }
}

impl From<DecodeError> for ReadApiError {
    fn from(source: DecodeError) -> Self {
        Self::Store {
            message: source.to_string(),
        }
    }
}
