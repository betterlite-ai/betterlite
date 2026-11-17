use http::HeaderMap;
use serde_json::error::Category;
use thiserror::Error;

use crate::response_content::ResponseContentError;

// LiteLLM hands a raw_response.json() failure and the upstream error branch the same AnthropicError, so those
// become ResponseError::Anthropic; a body of valid JSON with the wrong shape keeps a structural failure instead
#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("the response body is not JSON: {message}")]
    MalformedJson { message: String },
    #[error("the response body is not a JSON object")]
    NotAnObject,
    #[error("the response body has no content field")]
    MissingContent,
    #[error("the response body content field is not a list")]
    ContentType,
}

impl DecodeError {
    #[must_use]
    pub fn from_json(error: serde_json::Error) -> Self {
        match error.classify() {
            Category::Syntax | Category::Eof | Category::Io => Self::MalformedJson {
                message: error.to_string(),
            },
            Category::Data => Self::NotAnObject,
        }
    }
}

#[derive(Debug, Error)]
pub enum ResponseError {
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error("the Anthropic response field {0} is required")]
    MissingField(&'static str),
    #[error("the caller-built response has no choices[0] to fill")]
    MissingChoice,
    #[error("the Anthropic response field {field} has a type its declared shape does not allow")]
    FieldType { field: &'static str },
    #[error("the Anthropic request parameter {field} has a value its declared type does not allow")]
    Parameter { field: &'static str },
    #[error(transparent)]
    Content(#[from] ResponseContentError),
    // the counter's failure type belongs to the caller, so its text travels instead of a typed source
    #[error("the Anthropic usage could not be calculated: {message}")]
    Usage { message: String },
    #[error("{message}")]
    Anthropic {
        message: String,
        status: u16,
        headers: HeaderMap,
    },
}
