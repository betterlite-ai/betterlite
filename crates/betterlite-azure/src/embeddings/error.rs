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
    // the sync catch-all: status_code or 500, headers or response.headers, and response.text once the headers came from it
    #[must_use]
    pub fn wrapped(error: EmbeddingError) -> Self {
        match error {
            EmbeddingError::Azure(raised) => raised,
            EmbeddingError::Sdk(raised) => {
                let status_code = raised.status_code.unwrap_or(DEFAULT_STATUS);
                match (raised.headers, raised.response) {
                    (None, Some(response)) => Self {
                        status_code,
                        message: response.text,
                        headers: Some(response.headers),
                        cause: None,
                    },
                    (headers, _response) => Self {
                        status_code,
                        message: raised.message,
                        headers,
                        cause: None,
                    },
                }
            }
            EmbeddingError::Endpoint(raised) => Self {
                status_code: raised.status().unwrap_or(DEFAULT_STATUS),
                message: raised.to_string(),
                headers: None,
                cause: None,
            },
            EmbeddingError::Parse(raised) => Self {
                status_code: DEFAULT_STATUS,
                message: raised.to_string(),
                headers: None,
                cause: Some(raised),
            },
            EmbeddingError::Value { message } => Self {
                status_code: DEFAULT_STATUS,
                message,
                headers: None,
                cause: None,
            },
            EmbeddingError::Logging(raised) => Self {
                status_code: DEFAULT_STATUS,
                message: raised.to_string(),
                headers: None,
                cause: None,
            },
            EmbeddingError::Encode(raised) => Self {
                status_code: DEFAULT_STATUS,
                message: raised.to_string(),
                headers: None,
                cause: None,
            },
        }
    }
}
