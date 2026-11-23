use std::error::Error;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use rust_decimal::Decimal;
use serde::Deserialize;
use thiserror::Error;

use crate::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use crate::passthrough_stream_logging::object::StreamLoggingObject;

pub const PASSTHROUGH_LOGGING_PAYLOAD: &str = "passthrough_logging_payload";
pub const PAYLOAD_ERROR: &str = "Error creating LLM passthrough response logging payload";
pub const NO_COMPLETE_RESPONSE: &str = "Unable to build complete streaming response for Anthropic passthrough endpoint, not logging...";

pub(super) const MODEL: &str = "model";
pub(super) const RESPONSE_COST: &str = "response_cost";
pub(super) const LITELLM_PARAMS: &str = "litellm_params";
pub(super) const PROXY_SERVER_REQUEST: &str = "proxy_server_request";
pub(super) const BODY: &str = "body";
pub(super) const USER: &str = "user";

// httpx.Response, which LiteLLM's transform_response reads
#[derive(Clone, Copy, Debug)]
pub struct UpstreamChatResponse<'a> {
    pub status_code: u16,
    pub headers: &'a HeaderMap,
    pub body: &'a [u8],
}

// LiteLLM's total=False PassthroughStandardLoggingPayload; only request_body is read here
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PassthroughLoggingPayload {
    pub url: Option<String>,
    pub request_method: Option<String>,
    pub request_body: Option<JsonObject>,
    pub response_body: Option<JsonObject>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub cost_per_request: Option<Decimal>,
}

// the ModelResponse | TextCompletionResponse | None that a complete chunk rebuild returns
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamedResponse {
    Model(Json),
    TextCompletion(Json),
}

// the fields passthrough_chat_handler reads from its arguments
pub struct PassthroughChatInput<'a> {
    pub upstream: UpstreamChatResponse<'a>,
    pub response_body: &'a JsonObject,
    pub logging_obj: &'a StreamLoggingObject,
    pub request_body: &'a JsonObject,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub kwargs: PassThroughLoggingKwargs,
}

// what build_response_logging_payload writes: the response, the kwargs and the write destinations
pub struct ResponsePayloadInput<'a> {
    pub response: &'a mut Json,
    pub model: &'a str,
    pub kwargs: PassThroughLoggingKwargs,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub logging_obj: &'a StreamLoggingObject,
}

// the fields handle_collected_chunks reads from its arguments
pub struct CollectedChunksInput<'a> {
    pub logging_obj: &'a StreamLoggingObject,
    pub request_body: &'a JsonObject,
    pub all_chunks: &'a [String],
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
}

#[derive(Debug, Error)]
pub enum PassthroughHandlerError {
    #[error("the {field} value must be a string")]
    ModelType { field: &'static str },
    #[error("the transformed response is not a JSON object")]
    ResponseShape,
    #[error("the passthrough logging payload could not be encoded: {source}")]
    PassthroughPayloadJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the passthrough logging payload could not be read: {source}")]
    PassthroughPayload {
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
    #[error("the litellm_params entry is not a JSON object")]
    LitellmParamsShape,
    #[error("the logging state is not available")]
    StateUnavailable,
    #[error("the standard logging object could not be encoded: {source}")]
    StandardLoggingJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the response cost could not be written as a JSON number: {source}")]
    CostNumber {
        #[source]
        source: serde_json::Error,
    },
    #[error("{source}")]
    Leaf {
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
}

impl PassthroughHandlerError {
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
