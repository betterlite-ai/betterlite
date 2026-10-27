use std::fmt;
use thiserror::Error;
use tokio_postgres::error::SqlState;

#[derive(Error)]
pub enum DbError {
    #[error("No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys")]
    NoDbConnected,
    #[error("database pool unavailable")]
    Pool {
        #[source]
        source: deadpool_postgres::PoolError,
    },
    #[error("database driver error")]
    Backend {
        #[source]
        source: tokio_postgres::Error,
    },
    #[error("invalid database record")]
    InvalidRecord { message: String },
    #[error("create returned no model")]
    CreateNull,
    #[error("unique constraint violation: {constraint:?}")]
    UniqueViolation {
        constraint: Option<String>,
        #[source]
        source: tokio_postgres::Error,
    },
    #[error("database SQLSTATE {code:?}: {constraint:?}")]
    SqlState {
        code: SqlState,
        constraint: Option<String>,
        #[source]
        source: tokio_postgres::Error,
    },
}

// Driver sources retain diagnostics for policy, but may contain SQL parameters in their Debug output.
impl fmt::Debug for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

pub fn backend(source: tokio_postgres::Error) -> DbError {
    let constraint = source
        .as_db_error()
        .and_then(|error| error.constraint())
        .map(str::to_owned);
    match source.code() {
        Some(code) if code == &SqlState::UNIQUE_VIOLATION => {
            DbError::UniqueViolation { constraint, source }
        }
        Some(code) => DbError::SqlState {
            code: code.clone(),
            constraint,
            source,
        },
        None => DbError::Backend { source },
    }
}

#[must_use]
pub const fn is_unique_violation(error: &DbError) -> bool {
    match error {
        DbError::UniqueViolation { .. } => true,
        DbError::NoDbConnected
        | DbError::Pool { .. }
        | DbError::Backend { .. }
        | DbError::InvalidRecord { .. }
        | DbError::CreateNull
        | DbError::SqlState { .. } => false,
    }
}
