use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::error::{OpenAiError, ProviderError, ProviderErrorParts};
use betterlite_openai::response_events::RawResponse;
use betterlite_openai::response_events::ReplyBody;
use betterlite_openai::response_events::created_at::convert_created_at;
use indexmap::IndexMap;
use serde_json::Number;

use super::ChatGptResponsesError;
use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_sse_recovery::decode_recovery_line;
use crate::api::responses_sse_recovery::records::{
    OutputItems, TextOnlyItems, merged_output_items, record_output_item_chunk,
    record_output_text_chunk,
};
use crate::api::responses_stream_primitives::python_text::value_text;

const RESPONSE_COMPLETED: &str = "response.completed";
const OUTPUT_ITEM_DONE: &str = "response.output_item.done";
const OUTPUT_TEXT_DONE: &str = "response.output_text.done";
const RESPONSE_FAILED: &str = "response.failed";
const ERROR: &str = "error";
const CREATED_AT: &str = "created_at";

pub(super) struct Recovered {
    pub(super) response: Option<ReplyBody>,
    pub(super) error_message: Option<String>,
}

// _should_parse_as_sse: a content type says so, or the text carries a recorded event
pub(super) fn should_parse_as_sse(headers: &IndexMap<String, String>, body: &str) -> bool {
    let content_type = headers
        .iter()
        .find(|(name, _value)| name.eq_ignore_ascii_case("content-type"))
        .map_or("", |(_name, value)| value.as_str());
    if content_type
        .to_ascii_lowercase()
        .contains("text/event-stream")
    {
        return true;
    }
    let trimmed = body.trim_start();
    trimmed.starts_with("event:")
        || trimmed.starts_with("data:")
        || body.contains("\nevent:")
        || body.contains("\ndata:")
}

// _extract_completed_response_from_sse: the first completed event ends the search even when it holds no response
pub(super) fn extract_completed(
    body: &str,
    now: i64,
    limits: &ApiLimits,
) -> Result<Recovered, ChatGptResponsesError> {
    let mut output_items = OutputItems::new();
    let mut text_only_items = TextOnlyItems::new();
    let mut error_message = None;
    for line in body.split(['\r', '\n']) {
        let Some(parsed) = decode_recovery_line(line.as_bytes(), limits)
            .map_err(|source| ChatGptResponsesError::Recovery { source })?
        else {
            continue;
        };
        match parsed.get("type").and_then(event_type) {
            Some(RESPONSE_COMPLETED) => {
                let response =
                    build_completed(&parsed, &output_items, &text_only_items, now, limits)?;
                return Ok(Recovered {
                    response,
                    error_message,
                });
            }
            Some(OUTPUT_ITEM_DONE) => record_output_item_chunk(&parsed, &mut output_items, limits)
                .map_err(|source| ChatGptResponsesError::Recovery { source })?,
            Some(OUTPUT_TEXT_DONE) => {
                record_output_text_chunk(&parsed, &output_items, &mut text_only_items, limits)
                    .map_err(|source| ChatGptResponsesError::Recovery { source })?;
            }
            Some(RESPONSE_FAILED | ERROR) => {
                if let Some(failure) = error_message_of(&parsed) {
                    error_message = Some(failure);
                }
            }
            Some(_) | None => {}
        }
    }
    Ok(Recovered {
        response: None,
        error_message,
    })
}

fn event_type(value: &Json) -> Option<&str> {
    match value {
        Json::Text(kind) => Some(kind),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// _build_completed_response_from_chunk: only a missing output is recovered, and a payload no record holds is kept as sent
fn build_completed(
    parsed: &IndexMap<String, Json>,
    output_items: &OutputItems,
    text_only_items: &TextOnlyItems,
    now: i64,
    limits: &ApiLimits,
) -> Result<Option<ReplyBody>, ChatGptResponsesError> {
    let Some(Json::Object(given)) = parsed.get("response") else {
        return Ok(None);
    };
    let mut payload = given.clone();
    let missing_output = match payload.get("output") {
        Some(Json::List(items)) => items.is_empty(),
        None | Some(Json::Null) => true,
        Some(_) => false,
    };
    if missing_output {
        let merged = merged_output_items(output_items, text_only_items, limits)
            .map_err(|source| ChatGptResponsesError::Recovery { source })?;
        if !merged.is_empty() {
            payload.insert(String::from("output"), Json::List(merged));
        }
    }
    if let Some(created) = payload.get(CREATED_AT) {
        let timestamp = convert_created_at(created, now)?;
        payload.insert(
            String::from(CREATED_AT),
            Json::Number(Number::from(timestamp)),
        );
    }
    decode_completed(&payload).map(Some)
}

fn decode_completed(payload: &IndexMap<String, Json>) -> Result<ReplyBody, ChatGptResponsesError> {
    let value = serde_json::to_value(Json::Object(payload.clone()))
        .map_err(|source| ChatGptResponsesError::Encode { source })?;
    match serde_json::from_value::<ResponsesApiResponse>(value) {
        Ok(response) => Ok(ReplyBody::Typed(Box::new(response))),
        Err(error) => {
            tracing::debug!(
                error = %error,
                "the completed event did not validate; keeping its fields as sent"
            );
            Ok(ReplyBody::Lax(payload.clone()))
        }
    }
}

// _extract_error_message: the event's own error wins, then the failed response's, and the latest one is kept
fn error_message_of(parsed: &IndexMap<String, Json>) -> Option<String> {
    let error = match parsed.get("error") {
        Some(Json::Null) | None => match parsed.get("response") {
            Some(Json::Object(response)) => response.get("error"),
            None | Some(_) => None,
        },
        Some(error) => Some(error),
    }?;
    match error {
        Json::Null => None,
        value @ Json::Object(fields) => match fields.get("message") {
            Some(Json::Null) | None => Some(value_text(value)),
            Some(message) => Some(value_text(message)),
        },
        value @ Json::Bool(_)
        | value @ Json::Number(_)
        | value @ Json::Text(_)
        | value @ Json::List(_) => Some(value_text(value)),
    }
}

// a missing completion is an OpenAIError at the status the provider answered with
pub(super) fn missing_completion(
    reply: &RawResponse<'_>,
    error_message: Option<String>,
) -> OpenAiError {
    let text = error_message
        .filter(|candidate| !candidate.is_empty())
        .unwrap_or_else(|| reply.text.to_owned());
    OpenAiError::Provider(Box::new(ProviderError::from(ProviderErrorParts {
        status_code: reply.status,
        message: text,
        headers: None,
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    })))
}
