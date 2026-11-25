use std::sync::Arc;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use std::str::FromStr as _;

use crate::fields::usage::Usage;
use crate::fields::usage::parse::usage_from_mapping;
// ast-grep-ignore: no-long-names
use crate::logging_failure::partial_usage::record_partial_usage_for_failure;
use crate::logging_success::ports::SuccessClock;
use crate::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use crate::passthrough_logging_state::logger::PassThroughEndpointLogging;
use crate::passthrough_logging_state::response::PassThroughResponse;
use crate::passthrough_stream_logging::build::StreamLoggingInput;
// ast-grep-ignore: no-long-names
use crate::passthrough_stream_logging::build::build_passthrough_logging_result;
use crate::passthrough_stream_logging::object::StreamLoggingObject;
use crate::passthrough_stream_logging::types::{EndpointType, ProviderChunkHandlers};

use super::context::PassThroughStreamContext;

const USAGE_KEY: &str = "usage";
const RESPONSE_COST_KEY: &str = "response_cost";

// AnthropicPassthroughLoggingHandler.record_partial_usage_for_failure, owned by the provider-leaf task
pub trait AnthropicPartialUsage: Send + Sync {
    fn record_partial_usage(
        &self,
        logging_obj: &StreamLoggingObject,
        request_body: &Json,
        all_chunks: &[Vec<u8>],
    );
}

// Everything _record_partial_usage_for_failure reads, owned so the reconstruction runs off the reactor
pub struct PartialUsageCall {
    pub logging_obj: Arc<StreamLoggingObject>,
    pub passthrough: Arc<PassThroughEndpointLogging>,
    pub providers: Arc<ProviderChunkHandlers>,
    pub anthropic: Arc<dyn AnthropicPartialUsage>,
    pub clock: Arc<dyn SuccessClock>,
    pub endpoint_type: EndpointType,
    pub request_body: Json,
    pub raw_bytes: Vec<Vec<u8>>,
    pub stream_context: Option<PassThroughStreamContext>,
}

// LiteLLM's _record_partial_usage_for_failure: a failed stream still logs the usage its chunks carry
pub fn record_partial_usage_failure(call: &PartialUsageCall) {
    if call.endpoint_type == EndpointType::Anthropic {
        call.anthropic.record_partial_usage(
            call.logging_obj.as_ref(),
            &call.request_body,
            &call.raw_bytes,
        );
        return;
    }
    let Some(stream_context) = call.stream_context.as_ref() else {
        return;
    };
    if call.raw_bytes.is_empty() {
        return;
    }
    let input = usage_input(call, stream_context);
    // ast-grep-ignore: no-long-names
    let output = match build_passthrough_logging_result(&input) {
        Ok(output) => output,
        Err(error) => {
            tracing::warn!(
                "Could not recover the partial usage of a failed {} pass-through stream: {error}",
                endpoint_value(call.endpoint_type)
            );
            return;
        }
    };
    let Some(usage) = response_usage(&output.result) else {
        return;
    };
    let cost = match response_cost(&output.kwargs) {
        Ok(cost) => cost,
        Err(error) => {
            tracing::warn!("Could not convert the partial response cost to decimal: {error}");
            return;
        }
    };
    call.logging_obj
        // ast-grep-ignore: no-long-names
        .edit(&mut |state| record_partial_usage_for_failure(state, &usage, cost));
}

fn usage_input(
    call: &PartialUsageCall,
    stream_context: &PassThroughStreamContext,
) -> StreamLoggingInput {
    StreamLoggingInput {
        logging_obj: Arc::clone(&call.logging_obj),
        passthrough: Arc::clone(&call.passthrough),
        providers: Arc::clone(&call.providers),
        url_route: stream_context.url_route.clone(),
        request_body: Some(call.request_body.clone()),
        endpoint_type: call.endpoint_type,
        start_time: stream_context.start_time,
        end_time: call.clock.now(),
        raw_bytes: call.raw_bytes.clone(),
        model: None,
    }
}

// getattr(partial_response, "usage", None) and isinstance(usage, Usage): only the response models carry a Usage
fn response_usage(response: &PassThroughResponse) -> Option<Usage> {
    let value = match response {
        PassThroughResponse::ModelResponse(value)
        | PassThroughResponse::TextCompletion(value)
        | PassThroughResponse::Embedding(value) => value,
        PassThroughResponse::Image(_)
        | PassThroughResponse::VideoOperation(_)
        | PassThroughResponse::Standard(_)
        | PassThroughResponse::ResponsesApi(_)
        | PassThroughResponse::Transcription(_)
        | PassThroughResponse::VertexBatchRecord(_)
        | PassThroughResponse::VertexSearchWrapper(_) => return None,
    };
    let Json::Object(fields) = value else {
        return None;
    };
    let Json::Object(usage) = fields.get(USAGE_KEY)? else {
        return None;
    };
    usage_from_mapping(usage).ok()
}

fn response_cost(kwargs: &PassThroughLoggingKwargs) -> Result<Decimal, rust_decimal::Error> {
    let Some(Json::Number(number)) = kwargs.entries.get(RESPONSE_COST_KEY) else {
        return Ok(Decimal::ZERO);
    };
    let text = number.to_string();
    Decimal::from_str(&text).or_else(|_plain| Decimal::from_scientific(&text))
}

const fn endpoint_value(endpoint_type: EndpointType) -> &'static str {
    match endpoint_type {
        EndpointType::VertexAi => "vertex-ai",
        EndpointType::Gemini => "gemini",
        EndpointType::Anthropic => "anthropic",
        EndpointType::OpenAi => "openai",
        EndpointType::Tinyfish => "tinyfish",
        EndpointType::Generic => "generic",
        EndpointType::Decisions => "decisions",
    }
}

mod tests;
