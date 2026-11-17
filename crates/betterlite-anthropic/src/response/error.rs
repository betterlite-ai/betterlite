use std::convert::Infallible;
use std::error::Error as StdError;

use http::HeaderMap;
use serde_json::error::Category;
use thiserror::Error;

use crate::response_content::ResponseContentError;
use crate::usage::error::UsageError;

// A json() failure and the upstream error branch both raise AnthropicError; a wrong-shaped body stays structural
#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("the response body is not JSON: {source}")]
    MalformedJson {
        #[source]
        source: serde_json::Error,
    },
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
            Category::Syntax | Category::Eof | Category::Io => {
                Self::MalformedJson { source: error }
            }
            Category::Data => Self::NotAnObject,
        }
    }
}

#[derive(Debug, Error)]
pub enum ResponseError<CounterFailure = Infallible>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
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
    #[error("the Anthropic usage could not be calculated: {source}")]
    Usage {
        #[from]
        source: UsageError<CounterFailure>,
    },
    #[error("{message}")]
    Anthropic {
        message: String,
        status: u16,
        headers: HeaderMap,
    },
}
