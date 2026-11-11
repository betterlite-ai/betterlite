use std::sync::Arc;

use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};

use super::lines::convert_raw_bytes_to_str_lines;
use super::object::StreamLoggingObject;
use super::types::{
    CollectedChunksCall, EndpointType, HandlerError, HandlerOutcome, ProviderChunkHandlers,
};
use crate::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use crate::passthrough_logging_state::logger::PassThroughEndpointLogging;
use crate::passthrough_logging_state::response::{PassThroughResponse, PassThroughResponseObject};

pub struct StreamLoggingInput {
    pub logging_obj: Arc<StreamLoggingObject>,
    pub passthrough: Arc<PassThroughEndpointLogging>,
    pub providers: Arc<ProviderChunkHandlers>,
    pub url_route: String,
    pub request_body: Option<Json>,
    pub endpoint_type: EndpointType,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub raw_bytes: Vec<Vec<u8>>,
    pub model: Option<String>,
}

#[derive(Debug)]
pub struct StreamLoggingOutput {
    pub result: PassThroughResponse,
    pub kwargs: PassThroughLoggingKwargs,
}

// ast-grep-ignore: no-long-names
pub fn build_passthrough_logging_result(
    input: &StreamLoggingInput,
) -> Result<StreamLoggingOutput, HandlerError> {
    let lines = convert_raw_bytes_to_str_lines(&input.raw_bytes);
    let outcome =
        match input.endpoint_type {
            EndpointType::Anthropic => input
                .providers
                .anthropic
                .handle(&handler_call(input, &lines, None))?,
            EndpointType::VertexAi => input.providers.vertex.handle(&handler_call(
                input,
                &lines,
                input.model.as_deref(),
            ))?,
            EndpointType::Gemini => input.providers.gemini.handle(&handler_call(
                input,
                &lines,
                input.model.as_deref(),
            ))?,
            EndpointType::OpenAi => input
                .providers
                .openai
                .handle(&handler_call(input, &lines, None))?,
            EndpointType::Tinyfish | EndpointType::Generic | EndpointType::Decisions => {
                HandlerOutcome::default()
            }
        };
    Ok(StreamLoggingOutput {
        result: outcome.result.unwrap_or_else(|| fallback_response(&lines)),
        kwargs: outcome.kwargs,
    })
}

fn handler_call<'call>(
    input: &'call StreamLoggingInput,
    lines: &'call [String],
    model: Option<&'call str>,
) -> CollectedChunksCall<'call> {
    CollectedChunksCall {
        logging_obj: input.logging_obj.as_ref(),
        passthrough: input.passthrough.as_ref(),
        url_route: &input.url_route,
        request_body: input.request_body.as_ref(),
        endpoint_type: input.endpoint_type,
        start_time: input.start_time,
        end_time: input.end_time,
        lines,
        model,
    }
}

fn fallback_response(lines: &[String]) -> PassThroughResponse {
    // A list of strings always serializes, so the empty text from a failed encode cannot be produced.
    let rendered = serde_json::to_string(lines).unwrap_or_default();
    PassThroughResponse::Standard(PassThroughResponseObject {
        response: Json::Text(format!(
            "cannot parse chunks to standard response object. Chunks={rendered}"
        )),
    })
}
