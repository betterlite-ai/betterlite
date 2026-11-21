use betterlite_auth::identity::AgentIdentityFailure;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::errors::is_unique_violation;
use thiserror::Error;
use tokio_postgres::Error as PostgresError;

use crate::agent_credentials::error::AgentCredentialsError;
use crate::agent_rows::codec::RowDecodeError;
use crate::object_permissions::error::ObjectPermissionError;

pub(super) const REGISTRATION_CONFLICT: &str =
    "Agent name or Entra application is already registered";
pub(super) const APPLICATION_CONFLICT: &str =
    "Entra application was already registered to another agent";

#[derive(Debug, Error)]
pub enum AgentPersistenceError {
    #[error("{0}")]
    Conflict(&'static str),
    #[error("{}", .0.message)]
    Identity(AgentIdentityFailure),
    #[error("{0}")]
    NotFound(String),
    #[error(transparent)]
    Database(#[from] DbError<RowDecodeError>),
    #[error(transparent)]
    Credentials(#[from] AgentCredentialsError),
    #[error(transparent)]
    ObjectPermission(#[from] ObjectPermissionError),
    #[error("the agent JSON could not be built")]
    Json(#[from] serde_json::Error),
}

// a session error carries no record to decode, so its type widens to the one the row codec reports
pub(super) fn open_failure(error: DbError) -> AgentPersistenceError {
    let widened = match error {
        DbError::NoDbConnected => DbError::NoDbConnected,
        DbError::Pool { source } => DbError::Pool { source },
        DbError::Backend { source } => DbError::Backend { source },
        DbError::InvalidRecord { source } => match source {},
        DbError::CreateNull => DbError::CreateNull,
        DbError::UniqueViolation { constraint, source } => {
            DbError::UniqueViolation { constraint, source }
        }
        DbError::SqlState {
            code,
            constraint,
            source,
        } => DbError::SqlState {
            code,
            constraint,
            source,
        },
    };
    AgentPersistenceError::Database(widened)
}

// every unique index on these tables answers LiteLLM's one 409 text
pub(super) fn write_error(source: PostgresError) -> AgentPersistenceError {
    let error: DbError<RowDecodeError> = backend(source);
    if is_unique_violation(&error) {
        return AgentPersistenceError::Conflict(REGISTRATION_CONFLICT);
    }
    AgentPersistenceError::Database(error)
}
