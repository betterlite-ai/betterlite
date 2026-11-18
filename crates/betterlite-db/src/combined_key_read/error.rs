use crate::database_faults::DbError;

use super::view::ViewError;

pub const MISSING_TOKEN_STATUS: u16 = 400;
pub const MISSING_TOKEN_DETAIL: &str = "No token passed in. Token=None";
pub const READ_CALL_TYPE: &str = "get_data";

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ReconnectFailure {
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum KeyReadError {
    #[error("No token passed in. Token=None")]
    MissingToken,
    #[error(transparent)]
    Database(#[from] DbError),
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
