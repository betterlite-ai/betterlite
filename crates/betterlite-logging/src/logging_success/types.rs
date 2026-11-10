use std::sync::Arc;

use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use thiserror::Error;

use crate::callback_logger::event::GuardrailRequestSnapshot;
use crate::fields::payload::StandardLoggingPayload;

pub const PASS_THROUGH_CALL_TYPE: &str = "pass_through_endpoint";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SuccessResult {
    Chat(Json),
    Embeddings(Json),
    Chunk(Json),
    // a result betterlite does not model, logged only when LiteLLM accepts it as a dict or list
    Other(Json),
}

impl SuccessResult {
    #[must_use]
    pub const fn response(&self) -> &Json {
        match self {
            Self::Chat(response)
            | Self::Embeddings(response)
            | Self::Chunk(response)
            | Self::Other(response) => response,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuccessMode {
    Sync,
    Async,
}

// Everything LiteLLM's success_handler and async_success_handler receive besides the logging object
#[derive(Clone, Debug)]
pub struct SuccessRequest {
    pub result: SuccessResult,
    pub start_time: Option<DateTime<FixedOffset>>,
    pub end_time: Option<DateTime<FixedOffset>>,
    pub cache_hit: Option<bool>,
    pub prefer_async_handlers: bool,
    pub standard_logging_object: Option<StandardLoggingPayload>,
    pub guardrail_snapshot: Option<Arc<GuardrailRequestSnapshot>>,
}

#[derive(Debug, Error)]
pub enum SuccessError {
    #[error("the standard logging payload is not valid JSON: {source}")]
    PayloadJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the standard logging payload is not a JSON object")]
    PayloadShape,
}
