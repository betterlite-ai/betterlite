use betterlite_http::error::HttpError;
use thiserror::Error;

use crate::error::{ErrorRequest, ErrorResponse, ProviderError};
use crate::openai_base_config::Headers;

use super::API_BASE;

const REQUEST_METHOD: &str = "POST";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct BytezError {
    pub status_code: u16,
    pub message: String,
}

impl From<BytezError> for ProviderError {
    fn from(error: BytezError) -> Self {
        let request = ErrorRequest {
            method: REQUEST_METHOD.to_owned(),
            url: API_BASE.to_owned(),
        };
        Self {
            status_code: error.status_code,
            status_code_is_synthesized: false,
            message: error.message,
            headers: None,
            response: ErrorResponse {
                status_code: error.status_code,
                request: request.clone(),
            },
            request,
            body: None,
        }
    }
}

// LiteLLM's get_error_class builds the native error and never copies the response headers into it
#[must_use]
pub fn get_error_class(message: &str, status_code: u16, _headers: &Headers) -> BytezError {
    BytezError {
        status_code,
        message: message.to_owned(),
    }
}

#[derive(Debug, Error)]
pub enum BytezConfigError {
    #[error("the {key} param is not supported by Bytez")]
    UnsupportedParam { key: String },
    #[error("messages must be a non-empty array of OpenAI chat messages")]
    MissingMessages,
    #[error("the api key is required to call Bytez")]
    MissingApiKey,
    #[error("the message content must be a string, an object or a list of them")]
    ContentShape,
    #[error("a message content list holds a value that is not a string or an object")]
    ContentItemShape,
    #[error("the message content has no type")]
    MissingContentType,
    #[error("the message content type is not a Bytez content type")]
    UnknownContentType,
    #[error("the Bytez message content type {kind} is not supported")]
    UnsupportedContentType { kind: String },
    #[error("the message content {name} must be a non-empty string")]
    ContentValue { name: String },
    #[error("the Bytez token usage overflows")]
    UsageOverflow,
    #[error(transparent)]
    Url(#[from] HttpError),
}
