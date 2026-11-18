use std::error::Error as _;
use std::fmt;
use std::sync::Arc;

use crate::database_faults::{DbError, ErrorType};
use crate::postgres_store::diagnostics::DriverError;

use super::view::ViewError;

pub const MISSING_TOKEN_STATUS: u16 = 400;
pub const MISSING_TOKEN_DETAIL: &str = "No token passed in. Token=None";
pub const READ_CALL_TYPE: &str = "get_data";

#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct ReconnectFailure {
    pub message: String,
}

// a failed lookup keeps the native driver error typed, so a cached-plan match reads the database message
#[derive(Debug, Clone)]
pub enum DatabaseFailure {
    Reported(DbError),
    Driver(Arc<DriverError>),
}

impl DatabaseFailure {
    // the rendered form stays redacted: DriverError prints its SQLSTATE and constraint, never row values
    pub(crate) fn into_db_error(self) -> DbError {
        match self {
            Self::Reported(fault) => fault,
            Self::Driver(error) => database_fault(ErrorType::RawQuery, error.to_string()),
        }
    }
}

impl fmt::Display for DatabaseFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reported(fault) => fmt::Display::fmt(fault, f),
            Self::Driver(error) => fmt::Display::fmt(error, f),
        }
    }
}

// the rendered error keeps the reported fault's own cause chain and never the driver's raw detail
impl std::error::Error for DatabaseFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Reported(fault) => fault.source(),
            Self::Driver(_) => None,
        }
    }
}

impl From<DbError> for DatabaseFailure {
    fn from(error: DbError) -> Self {
        Self::Reported(error)
    }
}

pub(crate) fn database_fault(error_type: ErrorType, message: String) -> DbError {
    DbError {
        error_type,
        message,
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum KeyReadError {
    #[error("No token passed in. Token=None")]
    MissingToken,
    #[error(transparent)]
    Database(#[from] DatabaseFailure),
    #[error(transparent)]
    View(#[from] ViewError),
    #[error(transparent)]
    Reconnect(#[from] ReconnectFailure),
    #[error("the database metric failed: {message}")]
    Metrics { message: String },
}

impl KeyReadError {
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::MissingToken => "missing_token",
            Self::Database(_) => "database",
            Self::View(_) => "view",
            Self::Reconnect(_) | Self::Metrics { .. } => "other",
        }
    }

    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::MissingToken => Some(MISSING_TOKEN_STATUS),
            Self::Database(_) | Self::View(_) | Self::Reconnect(_) | Self::Metrics { .. } => None,
        }
    }

    #[must_use]
    pub const fn detail(&self) -> Option<&'static str> {
        match self {
            Self::MissingToken => Some(MISSING_TOKEN_DETAIL),
            Self::Database(_) | Self::View(_) | Self::Reconnect(_) | Self::Metrics { .. } => None,
        }
    }
}
