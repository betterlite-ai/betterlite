use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_sse_events::chunk::{ChunkFlag, DecodedMessagesChunk};
use betterlite_core::api::messages_sse_events::render::{
    incomplete_error_sse_event, response_as_sse_events,
};
use betterlite_core::api::messages_sse_events::{
    decode_messages_sse_chunk, decode_messages_sse_value,
};
use betterlite_models::decisions::Json;
use serde_json::json;

use crate::helpers::HelperError;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    action: Action,
    #[serde(default)]
    raw_bytes: Option<Vec<u8>>,
    #[serde(default)]
    value: Option<Json>,
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    response: Option<Json>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    RawChunkClassifiers,
    DataLineExtraction,
    #[serde(rename = "native_error_and_refusal_projection")]
    ErrorProjection,
    ResponseAsSseEvents,
    IncompleteErrorLiteral,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.action {
            Action::RawChunkClassifiers => Ok(classify(&decode(input)?)),
            Action::DataLineExtraction => extract(input, &decode(input)?),
            Action::ErrorProjection => Ok(project(&decode(input)?)),
            Action::ResponseAsSseEvents => render(input),
            Action::IncompleteErrorLiteral => literal(),
        }
    }
}

fn classify(chunk: &DecodedMessagesChunk) -> serde_json::Value {
    json!({
        "message_stop": chunk.flags.contains(&ChunkFlag::MessageStop),
        "ping": chunk.flags.contains(&ChunkFlag::Ping),
        "content_delta": chunk.flags.contains(&ChunkFlag::ContentDelta),
        "provider_error": chunk.flags.contains(&ChunkFlag::ProviderError),
        "terminal": chunk.flags.contains(&ChunkFlag::Terminal),
    })
}

fn extract(input: &Input, chunk: &DecodedMessagesChunk) -> crate::helpers::HelperResult {
    let event = input
        .event
        .as_deref()
        .ok_or_else(|| helper_error("ValueError", "the event field is required".to_owned()))?;
    serde_json::to_value(chunk.payload(event))
        .map_err(|error| helper_error("ValueError", error.to_string()))
}

fn project(chunk: &DecodedMessagesChunk) -> serde_json::Value {
    let error = chunk.messages_error().map(|error| {
        json!({
            "type": error.error_type,
            "message": error.message,
            "status": error.status,
        })
    });
    json!({
        "terminal": chunk.flags.contains(&ChunkFlag::Terminal),
        "error": error,
        "refusal_stop_details": chunk.refusal_stop_details(),
    })
}

fn render(input: &Input) -> crate::helpers::HelperResult {
    let response = input
        .response
        .as_ref()
        .ok_or_else(|| helper_error("ValueError", "the response field is required".to_owned()))?;
    let frames = response_as_sse_events(response)
        .map_err(|error| helper_error("ValueError", error.to_string()))?;
    let mut encoded = Vec::with_capacity(frames.len());
    for frame in frames {
        encoded.push(
            str::from_utf8(&frame)
                .map_err(|error| helper_error("UnicodeDecodeError", error.to_string()))?
                .to_owned(),
        );
    }
    Ok(json!(encoded))
}

fn literal() -> crate::helpers::HelperResult {
    let frame = incomplete_error_sse_event();
    let literal = str::from_utf8(&frame)
        .map_err(|error| helper_error("UnicodeDecodeError", error.to_string()))?;
    Ok(json!(literal))
}

fn decode(input: &Input) -> Result<DecodedMessagesChunk, Box<HelperError>> {
    if let Some(bytes) = &input.raw_bytes {
        return decode_messages_sse_chunk(bytes, &ApiLimits::default())
            .map_err(|error| helper_error("ValueError", error.to_string()));
    }
    Ok(decode_messages_sse_value(
        input.value.as_ref().unwrap_or(&Json::Null),
    ))
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
