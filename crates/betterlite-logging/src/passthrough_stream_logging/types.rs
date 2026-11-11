use std::error::Error;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use thiserror::Error;
use tokio::task::JoinError;

use super::object::StreamLoggingObject;
use crate::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use crate::passthrough_logging_state::logger::PassThroughEndpointLogging;
use crate::passthrough_logging_state::response::PassThroughResponse;

// LiteLLM's EndpointType for the stream router; betterlite-server's copy sits above this crate, so B102 maps one onto the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointType {
    VertexAi,
    Gemini,
    Anthropic,
    OpenAi,
    Tinyfish,
    Generic,
    Decisions,
}

pub struct CollectedChunksCall<'call> {
    pub logging_obj: &'call StreamLoggingObject,
    pub passthrough: &'call PassThroughEndpointLogging,
    pub url_route: &'call str,
    pub request_body: Option<&'call Json>,
    pub endpoint_type: EndpointType,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub lines: &'call [String],
    pub model: Option<&'call str>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HandlerOutcome {
    pub result: Option<PassThroughResponse>,
    pub kwargs: PassThroughLoggingKwargs,
}

#[derive(Debug, Error)]
pub enum HandlerError {
    #[error("{source}")]
    Leaf {
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    #[error("the collected chunks build task failed: {source}")]
    BlockingTask {
        #[source]
        source: JoinError,
    },
    #[error("the collected chunks build task returned no result")]
    BlockingTaskLost,
}

impl HandlerError {
    #[must_use]
    pub fn leaf<E>(source: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self::Leaf {
            source: Box::new(source),
        }
    }
}

pub trait CollectedChunksHandler: Send + Sync {
    fn handle(&self, call: &CollectedChunksCall<'_>) -> Result<HandlerOutcome, HandlerError>;
}

pub struct ProviderChunkHandlers {
    pub anthropic: Arc<dyn CollectedChunksHandler>,
    pub vertex: Arc<dyn CollectedChunksHandler>,
    pub gemini: Arc<dyn CollectedChunksHandler>,
    pub openai: Arc<dyn CollectedChunksHandler>,
}
