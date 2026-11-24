use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_models::decisions::Json;
use serde::Serialize;

use crate::health_access_and_connections::visibility::LookupFailure;
use crate::proxy_helpers::checks::{Detail, HttpException};

pub const NOT_AUTHORIZED: &str = "Not authorized to view this team's models";

// The two failures _filter_models_by_team_id and _find_model_by_id hand the route: the 403 guard, and a router index that ran off its rows.
#[derive(Debug, thiserror::Error)]
pub enum ScopeError {
    #[error("{NOT_AUTHORIZED}")]
    NotAuthorized,
    #[error(transparent)]
    Lookup(Box<LookupFailure>),
}

impl From<LookupFailure> for ScopeError {
    fn from(source: LookupFailure) -> Self {
        Self::Lookup(Box::new(source))
    }
}

// HTTPException's detail: the guard's {"error": message} map, or LiteLLM's string form of an unexpected error.
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
            Self::NotAuthorized => StatusCode::FORBIDDEN,
            Self::Lookup(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    #[must_use]
    pub fn detail(&self) -> ScopeDetail {
        match self {
            Self::NotAuthorized => ScopeDetail::Error {
                error: NOT_AUTHORIZED,
            },
            Self::Lookup(_) => ScopeDetail::Message(self.to_string()),
        }
    }
}

impl From<ScopeError> for HttpException {
    fn from(failure: ScopeError) -> Self {
        let text = match failure.detail() {
            ScopeDetail::Error { error: message } => message.to_owned(),
            ScopeDetail::Message(message) => message,
        };
        Self {
            status: failure.status_code(),
            detail: Some(Detail::Map(BTreeMap::from([(
                "error".to_owned(),
                Json::Text(text),
            )]))),
            message: None,
        }
    }
}
