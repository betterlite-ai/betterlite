use std::convert::Infallible;
use std::error::Error as StdError;

use thiserror::Error;

use crate::stream_state::error::StreamStateError;

// LiteLLM raises AnthropicError(status_code=500, message=...) for a native stream error event.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct AnthropicError {
    pub message: String,
    pub status_code: u16,
}

#[derive(Debug, Error)]
pub enum ShapeError {
    #[error("the Anthropic stream event must be a JSON object")]
    NotAnObject,
    #[error("the Anthropic stream event field {field} is required")]
    Missing { field: &'static str },
    #[error(
        "the Anthropic stream event field {field} has a type its declared shape does not allow"
    )]
    FieldType { field: &'static str },
    #[error("the Anthropic tool index overflowed")]
    IndexOverflow,
    #[error("the Anthropic stream event could not be encoded as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Anthropic server tool block assembled no argument text")]
    NoArguments,
    #[error("the Anthropic tool arguments could not be decoded as JSON")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Event(#[from] StreamStateError),
}

#[derive(Debug, Error)]
pub enum StreamEventError<CounterFailure = Infallible>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Shape(#[from] ShapeError),
    #[error(transparent)]
    Anthropic(#[from] AnthropicError),
    #[error("the Anthropic stream state could not be updated")]
    State {
        #[from]
        source: StreamStateError<CounterFailure>,
    },
}
