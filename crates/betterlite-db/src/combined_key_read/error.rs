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
#[derive(Debug, Clone, thiserror::Error)]
pub enum DatabaseFailure {
    // the reported fault spells its own Display and cause; a driver error hides its raw detail
    #[error(transparent)]
    Reported(#[from] DbError),
    #[error("{0}")]
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

pub(crate) const fn database_fault(error_type: ErrorType, message: String) -> DbError {
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
