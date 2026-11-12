use std::str::Utf8Error;

use betterlite_http::proxy::transport_error::TransportError;
use thiserror::Error;
use url::ParseError;

use crate::api::api_boundaries::ApiLimitError;

#[derive(Debug, Error)]
pub enum BetaConfigError {
    #[error("the beta headers config is too large")]
    TooLarge(#[source] ApiLimitError),
    #[error("the beta headers config is not UTF-8")]
    Utf8(#[source] Utf8Error),
    #[error("the beta headers config is not JSON")]
    Json(#[source] serde_json::Error),
    #[error("the beta headers config URL cannot be requested")]
    Url(#[source] ParseError),
    #[error("the beta headers config request failed")]
    Request(#[source] TransportError),
    #[error("the beta headers config server answered HTTP {status}")]
    Status { status: u16 },
    #[error("the bundled beta headers config is not a JSON object")]
    Bundled(#[source] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BetaShapeError {
    #[error("the beta headers config is a JSON {json_type}, not an object")]
    NotAnObject { json_type: &'static str },
    #[error("the beta headers config is empty")]
    Empty,
    #[error("the beta headers config names none of the expected providers")]
    MissingProviderKeys,
}

impl BetaShapeError {
    #[must_use]
    pub const fn kind(self) -> &'static str {
        match self {
            Self::NotAnObject { .. } => "not_a_dict",
            Self::Empty => "empty",
            Self::MissingProviderKeys => "missing_provider_keys",
        }
    }
}
