use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::aleph_alpha::error::AlephAlphaError;
use thiserror::Error;

use crate::langgraph_chat::error::LangGraphError;

#[derive(Debug, Error)]
pub enum LangGraphStreamError {
    #[error("the LangGraph stream {field} has the wrong type")]
    FieldType { field: &'static str },
    #[error(transparent)]
    Line(#[from] AlephAlphaError),
    #[error("the LangGraph stream body could not be read")]
    Body {
        #[source]
        source: HttpError,
    },
    #[error("the LangGraph stream timed out")]
    Timeout,
    #[error("the LangGraph stream error body is larger than {limit} bytes")]
    TooLarge { limit: usize },
    #[error("the LangGraph stream could not be reached")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the LangGraph stream request body could not be written as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the LangGraph stream URL is not a URL")]
    Url {
        #[source]
        source: url::ParseError,
    },
    #[error(transparent)]
    Provider(#[from] LangGraphError),
}
