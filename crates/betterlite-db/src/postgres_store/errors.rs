use std::fmt;
use thiserror::Error;
use tokio_postgres::error::SqlState;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("No DB Connected")]
    NoDbConnected,
    #[error("database pool error: {source}")]
    Pool {
        #[source]
        source: deadpool_postgres::PoolError,
    },
    #[error("database error: {source}")]
    Backend {
        #[source]
        source: tokio_postgres::Error,
    },
    #[error("invalid database record: {message}")]
    InvalidRecord { message: String },
    #[error("database invariant violated: {message}")]
    Invariant { message: String },
    #[error("unique constraint violation{constraint}")]
    UniqueViolation { constraint: ConstraintName },
    #[error("database SQLSTATE {code}{constraint}")]
    SqlState {
        code: String,
        constraint: ConstraintName,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConstraintName(pub Option<String>);

impl ConstraintName {
    pub(crate) fn from_error(error: &tokio_postgres::Error) -> Self {
        error
            .as_db_error()
            .and_then(|database| database.constraint())
            .map(|name| Self(Some(name.to_owned())))
            .unwrap_or_default()
    }
}

impl fmt::Display for ConstraintName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
            .as_ref()
            .map_or(Ok(()), |name| write!(f, " ({name})"))
    }
}

pub(crate) fn backend(error: tokio_postgres::Error) -> DbError {
    let constraint = ConstraintName::from_error(&error);
    match error.code() {
        Some(code) if code == &SqlState::UNIQUE_VIOLATION => {
            DbError::UniqueViolation { constraint }
        }
        Some(code) => DbError::SqlState {
            code: code.code().to_owned(),
            constraint,
        },
        None => DbError::Backend { source: error },
    }
}

#[must_use]
pub const fn is_unique_violation(error: &DbError) -> bool {
    matches!(error, DbError::UniqueViolation { .. })
}
