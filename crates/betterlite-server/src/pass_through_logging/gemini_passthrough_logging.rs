pub(crate) mod assembly;
pub mod cost;
mod payload;
pub(crate) mod relay;
mod reply;
mod routes;
pub(crate) mod tests;

use std::sync::Arc;

use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_vertex::model_policy::error::VertexAiError;
use betterlite_vertex::studio_veo_operation::error::StudioVeoOperationError;
use betterlite_vertex::vertex_relay_metadata::error::VertexRelayError;
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use serde_json::{Map, Value};
use thiserror::Error;

use self::cost::{PassthroughCompletionCost, PassthroughCostError};

pub struct GeminiPassthroughInput<'input> {
    pub logging_obj: &'input StreamLoggingObject,
    pub url_route: &'input str,
    pub response_headers: &'input HeaderMap,
    pub response_body: &'input [u8],
    pub request_body: &'input Map<String, Value>,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub kwargs: PassThroughLoggingKwargs,
}

#[derive(Debug, Error)]
pub enum GeminiPassthroughError {
    #[error("the Gemini response body could not be decoded: {source}")]
    Decode {
        #[from]
        source: VertexAiError,
    },
    #[error("the response cost of model {model} could not be resolved: {source}")]
    Cost {
        model: String,
        #[source]
        source: PassthroughCostError,
    },
    #[error(transparent)]
    Relay(#[from] VertexRelayError),
    #[error(transparent)]
    Video(#[from] StudioVeoOperationError),
    #[error("the request body could not be read as the Vertex JSON type: {source}")]
    RequestBody {
        #[source]
        source: serde_json::Error,
    },
    #[error("the logged Gemini response could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the collected Gemini chunks could not be assembled: {source}")]
    Collected {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl From<assembly::CollectedStreamError> for GeminiPassthroughError {
    fn from(source: assembly::CollectedStreamError) -> Self {
        Self::Collected {
            source: Box::new(source),
        }
    }
}

pub struct GeminiPassthroughLogger {
    cost: Arc<dyn PassthroughCompletionCost>,
}

impl GeminiPassthroughLogger {
    // ast-grep-ignore: no-one-line-functions - B8931 constructs the logger through this fixed name
    pub fn new(cost: Arc<dyn PassthroughCompletionCost>) -> Self {
        Self { cost }
    }
}
