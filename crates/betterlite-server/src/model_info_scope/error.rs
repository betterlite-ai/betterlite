use std::error::Error as StdError;

use axum::http::StatusCode;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use betterlite_router::model_catalog_lookup::error::LookupError;
use betterlite_router::model_group_lookups::error::AccessGroupError;
use serde::Serialize;
use thiserror::Error;

pub const USER_NOT_FOUND: &str = "User not found";

#[derive(Debug, Error)]
pub enum ScopeError {
    #[error("{}", CommonProxyErrors::DbNotConnectedError.as_str())]
    DatabaseUnavailable,
    #[error("{USER_NOT_FOUND}")]
    UserNotFound,
    #[error(transparent)]
    Lookup(#[from] LookupError),
    #[error(transparent)]
    AccessGroups(#[from] AccessGroupError),
    #[error("the caller's key could not be read")]
    Caller(#[source] serde_json::Error),
    #[error("the stored model row {model_id} could not be read")]
    ModelRead {
        model_id: String,
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },
    #[error("the stored user row could not be read")]
    UserRead(#[source] Box<dyn StdError + Send + Sync>),
    #[error("a stored team row could not be read")]
    TeamRead(#[source] Box<dyn StdError + Send + Sync>),
    #[error("the stored access group rows could not be read")]
    AccessGroupRead(#[source] Box<dyn StdError + Send + Sync>),
    #[error("the stored {field} grant holds a model name that is not text")]
    InvalidGrant { field: &'static str },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ScopeDetail {
    Error { error: &'static str },
    Message(String),
}

impl ScopeError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::UserNotFound => StatusCode::BAD_REQUEST,
            Self::DatabaseUnavailable
            | Self::Lookup(_)
            | Self::AccessGroups(_)
            | Self::Caller(_)
            | Self::ModelRead { .. }
            | Self::UserRead(_)
            | Self::TeamRead(_)
            | Self::AccessGroupRead(_)
            | Self::InvalidGrant { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    #[must_use]
    pub fn detail(&self) -> ScopeDetail {
        if let Self::DatabaseUnavailable = self {
            return ScopeDetail::Error {
                error: CommonProxyErrors::DbNotConnectedError.as_str(),
            };
        }
        if let Self::UserNotFound = self {
            return ScopeDetail::Error {
                error: USER_NOT_FOUND,
            };
        }
        ScopeDetail::Message(self.to_string())
    }
}
