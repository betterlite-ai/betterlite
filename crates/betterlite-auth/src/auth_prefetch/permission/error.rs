use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_faults::native::is_native_transport_error;
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_db::postgres_store::errors::DbError as StoreError;

use super::cache::PermissionCacheError;

// litellm/proxy/auth/auth_checks.py: get_object_permission's caller-visible 403 text
pub const MISSING_PERMISSION_MESSAGE: &str = "Referenced object permission does not exist";
pub const MISSING_PERMISSION_STATUS: u16 = 403;

#[derive(Debug, thiserror::Error)]
pub enum PermissionError {
    #[error("No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys")]
    NoDatabase,
    #[error("the object permission cache failed")]
    Cache(#[source] PermissionCacheError),
    #[error("Referenced object permission does not exist")]
    MissingInDatabaseOnly,
    #[error(transparent)]
    Lookup(#[from] DbLookupError),
    #[error("the object permission repository failed")]
    Repository(#[source] StoreError<TableError>),
    #[error("the object permission row did not validate")]
    Validation(#[source] serde_json::Error),
}

impl PermissionError {
    // only the database-only 403 carries a status; every other failure reaches its own handler
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::MissingInDatabaseOnly => Some(MISSING_PERMISSION_STATUS),
            Self::NoDatabase
            | Self::Cache(_)
            | Self::Lookup(_)
            | Self::Repository(_)
            | Self::Validation(_) => None,
        }
    }

    // B715's decorator records a DB failure only for the faults it can name, so pass it those
    #[must_use]
    pub fn database_fault(&self) -> Option<DbError> {
        match self {
            Self::Lookup(DbLookupError::Database(error)) => Some(error.clone()),
            Self::Repository(error) => repository_fault(error),
            Self::NoDatabase
            | Self::Cache(_)
            | Self::MissingInDatabaseOnly
            | Self::Lookup(
                DbLookupError::Deadline(_) | DbLookupError::Elapsed(_) | DbLookupError::Task(_),
            )
            | Self::Validation(_) => None,
        }
    }
}

// LiteLLM's prisma client raises a plain Exception for a missing client and PrismaError otherwise
fn repository_fault(error: &StoreError<TableError>) -> Option<DbError> {
    if matches!(error, StoreError::NoDbConnected) {
        return None;
    }
    let error_type = if is_native_transport_error(error) {
        ErrorType::Connect
    } else {
        ErrorType::Prisma
    };
    Some(DbError {
        error_type,
        message: error.to_string(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    })
}
