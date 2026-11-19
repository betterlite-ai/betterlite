use betterlite_anthropic::api::messages_errors::AnthropicExceptionMapping;
use betterlite_anthropic::api::messages_errors::decode::WireDecodeLimits;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use unicode_general_category::{GeneralCategory, get_general_category};

use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_options::INCOMPLETE_STREAM_MESSAGE;
use crate::api::request_form::scalars::scalar_int;
use crate::api::responses_stream_errors::failure::{FailureCause, FailureKind};
use crate::api::responses_stream_primitives::error_fields::{
    StreamErrorAttributes, StreamErrorObject, error_status_and_message,
};

use super::MessageStreamState;
use super::StreamFlag;
use super::error::MessagesResponseEventError;
use super::input::{ErrorAttribute, InvalidFailedResponse, ParsedEvent, text};
use super::log::StreamLog;

// _original_failure walks MidStreamFallbackError.original_exception while the wrapper still wraps one.
pub(super) fn original_failure(mut failure: &FailureCause) -> &FailureCause {
    loop {
        match failure.kind {
            FailureKind::Plain => return failure,
            FailureKind::MidStreamFallback => {
                let Some(original) = failure.original_exception.as_deref() else {
                    return failure;
                };
                failure = original;
            }
        }
    }
}

// _failure_status_and_message: a validated HTTP status and a nonempty message, with the pinned zero-status and message fallbacks.
#[must_use]
pub fn failure_status_and_message(failure: &FailureCause) -> (u16, String) {
    let original = original_failure(failure);
    let status_code = original
        .status_code
        .as_ref()
        .and_then(http_status)
        .unwrap_or(500);
    let message = match original.message.as_ref().and_then(text) {
        Some(message) if !message.is_empty() => message.to_owned(),
        Some(_) | None if !original.text.is_empty() => original.text.clone(),
        Some(_) | None => INCOMPLETE_STREAM_MESSAGE.to_owned(),
    };
    (status_code, message)
}

// _UpstreamFailure.http_error_status_or_none accepts a real int or a decimal string in 400..=599 and nothing else.
fn http_status(value: &Json) -> Option<u16> {
    let candidate = match value {
        Json::Number(number) => {
            let text = number.to_string();
            if text.contains(['.', 'e', 'E']) {
                return None;
            }
            text.parse::<u16>().ok()
        }
        Json::Text(text) if is_decimal(text) => u16::try_from(scalar_int(text)?.as_u64()?).ok(),
        Json::Bool(_) | Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    };
    candidate.filter(|status| (400..=599).contains(status))
}

// CPython's str.isdecimal(): non-empty, every character in category Decimal_Number.
fn is_decimal(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|character| get_general_category(character) == GeneralCategory::DecimalNumber)
}

// _anthropic_error_chunk: redact the client-visible message, then hand it to B958's Anthropic error mapper.
pub fn anthropic_error_chunk(
    status_code: u16,
    message: &str,
    redactor: Option<&Redactor>,
    limits: ApiLimits,
) -> Result<Json, MessagesResponseEventError> {
    let raw_message = redactor.map_or_else(
        || message.to_owned(),
        |scrubber| scrubber.redact_internal_details(message),
    );
    let record = AnthropicExceptionMapping::transform_to_anthropic_error(
        status_code,
        &raw_message,
        None,
        WireDecodeLimits {
            max_json_bytes: limits.max_json_bytes,
            max_event_bytes: limits.max_event_bytes,
            max_items: limits.max_items,
        },
    )
    .map_err(MessagesResponseEventError::ErrorEnvelope)?;
    serde_json::to_value(&record)
        .and_then(serde_json::from_value::<Json>)
        .map_err(MessagesResponseEventError::Json)
}

// The exception path of __anext__: the parsed failure chain becomes the terminal error chunk.
pub fn fail_with(
    state: &mut MessageStreamState,
    failure: &FailureCause,
) -> Result<(), MessagesResponseEventError> {
    let (status_code, message) = failure_status_and_message(failure);
    fail_stream(state, status_code, &message)
}

pub(super) fn failed(
    state: &mut MessageStreamState,
    event: ParsedEvent<'_>,
) -> Result<(), MessagesResponseEventError> {
    let error = event
        .failed_error()
        .map_err(|InvalidFailedResponse| MessagesResponseEventError::FailedEvent)?;
    let stream_error = match error {
        Some(ErrorAttribute::Mapping(fields)) => StreamErrorObject::Mapping(fields),
        Some(ErrorAttribute::Attributes(attributes)) => StreamErrorObject::Attributes(attributes),
        None => StreamErrorObject::Attributes(StreamErrorAttributes::default()),
    };
    let (status_code, message) = error_status_and_message(stream_error);
    let text = format!(
        "AnthropicResponsesStreamWrapper: upstream Responses stream for {} failed ({status_code}): {message}",
        state.model
    );
    tracing::error!("{text}");
    state.logs.push(StreamLog {
        level: "error",
        message: text,
    });
    fail_stream(state, status_code, &message)
}

fn fail_stream(
    state: &mut MessageStreamState,
    status_code: u16,
    message: &str,
) -> Result<(), MessagesResponseEventError> {
    state.stream_failed = StreamFlag(true);
    let chunk = anthropic_error_chunk(
        status_code,
        message,
        state.redactor.as_deref(),
        state.limits,
    )?;
    state.chunk_queue.push_back(chunk);
    Ok(())
}
