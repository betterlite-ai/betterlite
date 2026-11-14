use std::error::Error;
use std::num::TryFromIntError;

use betterlite_core::agents::provider::AgentProviderError;
use thiserror::Error;

use crate::a2a_text::request_utils::RequestUtilsError;
use crate::client::retry::SendError;
use crate::completion_bridge::dispatch::BridgeError;
use crate::errors::A2AError;
use crate::logging_context::LoggingContextError;

#[derive(Debug, Error)]
pub enum SendCallError {
    #[error("the A2A litellm_params field '{field}' must be a string")]
    NotText { field: &'static str },
    #[error("the A2A {value} could not be read as JSON")]
    Json {
        value: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Usage(#[from] RequestUtilsError),
    #[error("the A2A usage token count does not fit a u64")]
    TokenCount(#[source] TryFromIntError),
    #[error("Either a2a_client or api_base is required for standard A2A flow")]
    MissingClient,
    #[error("the A2A client could not be created")]
    Client(#[source] A2AError),
    #[error(transparent)]
    Send(#[from] SendError),
    #[error(transparent)]
    Logging(#[from] LoggingContextError),
    #[error("the A2A response could not be wrapped")]
    Response(#[source] AgentProviderError),
}

// The bridge handler has its own error type per completion and provider, so the entry point adds it to the call errors
#[derive(Debug, Error)]
pub enum SendEntryError<C, P>
where
    C: Error + 'static,
    P: Error + 'static,
{
    #[error(transparent)]
    Call(#[from] SendCallError),
    #[error(transparent)]
    Bridge(#[from] BridgeError<C, P>),
}
