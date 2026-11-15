use std::convert::Infallible;

use crate::key_repository::decode::TokenDecodeError;
use crate::postgres_store::errors::DbError;
use crate::user_repository::RowDecodeError;

use super::decode::ToolRecordDecodeError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ToolRecordsError {
    #[error("the tool table query failed")]
    Tools {
        #[source]
        source: DbError<ToolRecordDecodeError>,
    },
    #[error("the owner token lookup failed")]
    Tokens {
        #[source]
        source: DbError<TokenDecodeError>,
    },
    #[error("the owner user lookup failed")]
    Users {
        #[source]
        source: DbError<RowDecodeError>,
    },
    #[error("the tool override lookup failed")]
    Overrides {
        #[source]
        source: DbError<Infallible>,
    },
}

impl ToolRecordsError {
    // LiteLLM's owner lookup catches only prisma's database error, so a row that fails validation still escapes it
    pub(crate) fn is_database_failure(&self) -> bool {
        match self {
            Self::Tools { source } => is_driver_failure(source),
            Self::Tokens { source } => is_driver_failure(source),
            Self::Users { source } => is_driver_failure(source),
            Self::Overrides { source } => is_driver_failure(source),
        }
    }
}

fn is_driver_failure<Failure>(error: &DbError<Failure>) -> bool {
    match error {
        DbError::NoDbConnected
        | DbError::Pool { .. }
        | DbError::Backend { .. }
        | DbError::UniqueViolation { .. }
        | DbError::SqlState { .. } => true,
        DbError::InvalidRecord { .. } | DbError::CreateNull => false,
    }
}
