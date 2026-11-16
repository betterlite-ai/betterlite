use betterlite_logging::logging_pre_post::error::PrePostError;
use thiserror::Error;

use crate::endpoints::initialize::EndpointError;

use super::client::{Headers, ParseError, SdkError};

const DEFAULT_STATUS: u16 = 500;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct AzureOpenAiError {
    pub status_code: u16,
    pub message: String,
    pub headers: Option<Headers>,
    #[source]
    pub cause: Option<ParseError>,
}

#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error(transparent)]
    Azure(#[from] AzureOpenAiError),
    #[error("{message}")]
    Value { message: String },
    // the bare Exception that convert_to_model_response_object raises for an "error" in the answer: str() of it is empty
    #[error("")]
    Response { status_code: u16, message: String },
    #[error(transparent)]
    Sdk(#[from] SdkError),
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error(transparent)]
    Endpoint(#[from] EndpointError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error("the response could not be written for the log: {0}")]
    Encode(#[source] serde_json::Error),
}

impl AzureOpenAiError {
    #[must_use]
    pub const fn new(status_code: u16, message: String) -> Self {
        Self {
            status_code,
            message,
            headers: None,
            cause: None,
        }
    }

    fn from_sdk(raised: SdkError, message: String) -> Self {
        let (shown, headers) = match (raised.headers, raised.response) {
            (None, Some(response)) => (response.text, Some(response.headers)),
            (headers, _response) => (message, headers),
        };
        Self {
            headers,
            ..Self::new(raised.status_code.unwrap_or(DEFAULT_STATUS), shown)
        }
    }

    // the sync catch-all: status_code or 500, headers or response.headers, and response.text once the headers came from it
    #[must_use]
    pub fn wrapped(error: EmbeddingError) -> Self {
        let message = error.to_string();
        match error {
            EmbeddingError::Azure(raised) => raised,
            EmbeddingError::Sdk(raised) => Self::from_sdk(raised, message),
            EmbeddingError::Parse(raised) => Self {
                cause: Some(raised),
                ..Self::new(DEFAULT_STATUS, message)
            },
            EmbeddingError::Endpoint(raised) => {
                Self::new(raised.status().unwrap_or(DEFAULT_STATUS), message)
            }
            EmbeddingError::Response { status_code, .. } => Self::new(status_code, message),
            EmbeddingError::Value { .. }
            | EmbeddingError::Logging(_)
            | EmbeddingError::Encode(_) => Self::new(DEFAULT_STATUS, message),
        }
    }
}
