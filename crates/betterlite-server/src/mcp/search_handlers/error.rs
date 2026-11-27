use std::convert::Infallible;

use http::StatusCode;
use thiserror::Error;

use crate::management_rbac::FeatureAccessError;
use crate::mcp::tool_search::identity::MCPProxyToolIdentityError;

use super::ports::SearchPortError;

#[derive(Debug, Error)]
pub enum SearchHandlerError {
    #[error(transparent)]
    Feature(#[from] FeatureAccessError<Infallible>),
    #[error("{detail}")]
    Http { status: StatusCode, detail: String },
    #[error(transparent)]
    Port(#[from] SearchPortError),
    #[error(transparent)]
    Identity(#[from] MCPProxyToolIdentityError),
    #[error(transparent)]
    Serialize(#[from] serde_json::Error),
}

impl SearchHandlerError {
    #[must_use]
    pub fn forbidden(detail: &str) -> Self {
        Self::Http {
            status: StatusCode::FORBIDDEN,
            detail: detail.to_owned(),
        }
    }

    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Feature(error) => error.status_code(),
            Self::Http { status, .. } => *status,
            Self::Port(_) | Self::Identity(_) | Self::Serialize(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    #[must_use]
    pub fn detail(&self) -> String {
        match self {
            Self::Feature(error) => error.to_string(),
            Self::Http { detail, .. } => detail.clone(),
            Self::Port(error) => error.detail.clone(),
            Self::Identity(error) => error.to_string(),
            Self::Serialize(error) => error.to_string(),
        }
    }
}
