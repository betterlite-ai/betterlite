use super::diagnostics::{DriverError, PoolFailure};
use std::{convert::Infallible, fmt};
use thiserror::Error;
use tokio_postgres::error::SqlState;

#[derive(Error)]
pub enum DbError<E = Infallible> {
    #[error("No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys")]
    NoDbConnected,
    #[error("database pool unavailable")]
    Pool {
        #[source]
        source: PoolFailure,
    },
    #[error("database driver error")]
    Backend {
        #[source]
        source: DriverError,
    },
    #[error("invalid database record")]
    InvalidRecord {
        #[source]
        source: E,
    },
    #[error("create returned no model")]
    CreateNull,
    #[error("unique constraint violation: {constraint:?}")]
    UniqueViolation {
        constraint: Option<String>,
        #[source]
        source: DriverError,
    },
    #[error("database SQLSTATE {code:?}: {constraint:?}")]
    SqlState {
        code: SqlState,
        constraint: Option<String>,
        #[source]
        source: DriverError,
    },
}

impl<E> fmt::Debug for DbError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

pub fn backend<E>(driver: tokio_postgres::Error) -> DbError<E> {
    let source = DriverError { driver };
    let constraint = source
        .driver
        .as_db_error()
        .and_then(|error| error.constraint())
        .map(str::to_owned);
    match source.driver.code() {
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
pub const fn is_unique_violation<E>(error: &DbError<E>) -> bool {
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
