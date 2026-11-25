mod batch;
mod chunks;
mod payload;
mod tests;
mod transform;

use std::sync::Arc;

use betterlite_anthropic::batches::result::ResultError;
use betterlite_anthropic::response::error::ResponseError;
use betterlite_config::sections::request_headers::Settings;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::types::{HandlerError, HandlerOutcome};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_tokenizer::engine::error::EngineError;
use chrono::{DateTime, Utc};
use http::HeaderMap;
use serde::Serialize;
use thiserror::Error;

use self::payload::AnthropicPayloadInput;
use super::anthropic_batch::AnthropicBatchLogging;
use super::batch_attribution::optional_str;
use super::gemini_passthrough_logging::counter::CollectedCounter;
use super::interrupted_stream_usage::InterruptedStreamUsageError;
use super::response_costing::{ResponseCostService, cost_relevant_speed};
use super::stream_fields::StreamFieldError;

const BATCHES_ROUTE: &str = "/v1/messages/batches";
const SUCCESS_STATUS: u16 = 200;

// response_text is httpx_response.text; response_body is that body as an object, or empty for a list or no body
pub struct AnthropicPassthroughInput<'input> {
    pub status_code: u16,
    pub response_headers: &'input HeaderMap,
    pub response_text: &'input str,
    pub response_body: &'input JsonObject,
    pub url_route: &'input str,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub request_body: &'input JsonObject,
    pub kwargs: PassThroughLoggingKwargs,
}

#[derive(Debug, Error)]
pub enum AnthropicPassthroughError {
    #[error("the Anthropic response could not be transformed: {source}")]
    ChatTransform {
        #[source]
        source: ResponseError<EngineError>,
    },
    #[error("the Anthropic batch results could not be transformed: {source}")]
    BatchResults {
        #[source]
        source: ResultError<EngineError>,
    },
    #[error("the streamed Anthropic response could not be built: {source}")]
    StreamBuild {
        #[from]
        source: InterruptedStreamUsageError,
    },
    #[error("the Anthropic message_start event could not be read: {source}")]
    MessageStart {
        #[from]
        source: StreamFieldError,
    },
    #[error("the bundled tokenizer or the log redactor could not be built")]
    Tools,
    #[error("the logged Anthropic response could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the call log is held by the success dispatch")]
    CallLogHeld,
}

impl From<AnthropicPassthroughError> for HandlerError {
    fn from(source: AnthropicPassthroughError) -> Self {
        Self::leaf(source)
    }
}

// Both compile once with the logger, so no request rebuilds the vocabulary or the patterns.
struct TransformTools {
    counter: CollectedCounter,
    redactor: Redactor,
}

pub struct AnthropicPassthroughLogger {
    batch_logging: AnthropicBatchLogging,
    cost: Arc<dyn ResponseCostService>,
    tools: Option<TransformTools>,
    // The end-user lookup reads no request headers here, so the header settings stay at their defaults.
    request_headers: Settings,
}

impl AnthropicPassthroughLogger {
    #[must_use]
    pub fn new(batch_logging: AnthropicBatchLogging, cost: Arc<dyn ResponseCostService>) -> Self {
        Self {
            batch_logging,
            cost,
            tools: transform_tools(),
            request_headers: Settings::default(),
        }
    }

    // LiteLLM's order: the batch branch, then the transform get_anthropic_config picks, then the payload step.
    pub fn anthropic_passthrough_handler(
        &self,
        input: AnthropicPassthroughInput<'_>,
        call_log: &mut LoggingState,
    ) -> Result<HandlerOutcome, AnthropicPassthroughError> {
        if input.url_route.contains(BATCHES_ROUTE) && input.status_code == SUCCESS_STATUS {
            return batch::batch_outcome(&self.batch_logging, input, call_log);
        }
        let model = input
            .response_body
            .get("model")
            .and_then(optional_str)
            .unwrap_or_default()
            .to_owned();
        let speed = cost_relevant_speed(Some(input.request_body));
        let tools = self
            .tools
            .as_ref()
            .ok_or(AnthropicPassthroughError::Tools)?;
        let mut response =
            transform::transformed_response(tools, &input, speed.as_deref(), call_log)?;
        let response_id = input.response_body.get("id").and_then(optional_str);
        let kwargs = self.create_anthropic_response_logging_payload(AnthropicPayloadInput {
            response: &mut response,
            model: &model,
            kwargs: input.kwargs,
            call_log,
            response_id,
        });
        Ok(HandlerOutcome {
            result: Some(PassThroughResponse::ModelResponse(response)),
            kwargs,
        })
    }
}

fn transform_tools() -> Option<TransformTools> {
    let Some(counter) = CollectedCounter::bundled() else {
        tracing::error!("Anthropic passthrough logging: the bundled cl100k tokenizer is missing");
        return None;
    };
    match Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH) {
        Ok(redactor) => Some(TransformTools { counter, redactor }),
        Err(error) => {
            tracing::error!(
                "Anthropic passthrough logging: the log redactor failed to build: {error}"
            );
            None
        }
    }
}

// The logged response is the JSON form of the typed response each collaborator returns.
fn logged_json<T>(value: &T) -> Result<Json, AnthropicPassthroughError>
where
    T: Serialize,
{
    serde_json::to_value(value)
        .and_then(serde_json::from_value)
        .map_err(|source| AnthropicPassthroughError::Encode { source })
}
