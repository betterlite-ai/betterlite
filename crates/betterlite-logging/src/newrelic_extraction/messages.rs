use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::fields::timestamps::Moment;
use crate::langfuse_projections::steering::truthiness;
use crate::newrelic_extraction::ExtractionError;
use crate::newrelic_extraction::content::ContentPolicy;
use crate::newrelic_extraction::sources::EventSources;
use crate::newrelic_extraction::timing::{milliseconds_as_int, to_epoch_ms};
use crate::python_json;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageEvent {
    pub role: String,
    pub sequence: usize,
    pub response_model: String,
    pub vendor: String,
    pub is_response: Option<bool>,
    pub content: Option<String>,
    pub timestamp: Option<i64>,
}

struct MessageStyle<'event> {
    response_model: &'event str,
    vendor: &'event str,
    timestamp: Option<i64>,
    include_content: bool,
}

#[must_use]
pub fn message_content(message: &IndexMap<String, Json>) -> String {
    if let Some(tool_calls) = message.get("tool_calls").filter(|value| truthiness(value)) {
        return python_json::dumps(tool_calls);
    }
    let Some(content) = message.get("content") else {
        return String::new();
    };
    match content {
        Json::Text(text) => text.clone(),
        Json::Null => String::new(),
        Json::List(_) | Json::Bool(_) | Json::Number(_) | Json::Object(_) => {
            python_json::dumps(content)
        }
    }
}

pub fn all_messages(
    sources: &EventSources<'_>,
    response_model: &str,
    vendor: &str,
    policy: &ContentPolicy<'_>,
) -> Result<Vec<MessageEvent>, ExtractionError> {
    let payload_start = sources
        .standard_logging_object
        .map(|payload| payload.start_time);
    let payload_end = sources
        .standard_logging_object
        .map(|payload| payload.end_time);
    let start_time = preferred_time(payload_start, sources.kwargs.start_time);
    let end_time = preferred_time(payload_end, sources.kwargs.end_time);
    let start_timestamp = match start_time {
        Some(moment) => Some(milliseconds_as_int(to_epoch_ms(&moment)?)?),
        None => None,
    };
    let end_timestamp = match end_time {
        Some(moment) => Some(milliseconds_as_int(to_epoch_ms(&moment)?)?),
        None => None,
    };
    let include_content = policy.record_content() && !policy.redact_message_logging;
    let request_style = MessageStyle {
        response_model,
        vendor,
        timestamp: start_timestamp,
        include_content,
    };
    let response_style = MessageStyle {
        response_model,
        vendor,
        timestamp: end_timestamp,
        include_content,
    };
    let mut messages = request_events(sources, &request_style)?;
    let first_sequence = messages.len();
    messages.extend(response_events(
        sources.response,
        &response_style,
        first_sequence,
    )?);
    Ok(messages)
}

const fn preferred_time(
    payload_seconds: Option<Decimal>,
    kwargs_time: Option<Moment>,
) -> Option<Moment> {
    match payload_seconds {
        Some(seconds) if !seconds.is_zero() => Some(Moment::Float(seconds)),
        Some(_) | None => kwargs_time,
    }
}

fn request_events(
    sources: &EventSources<'_>,
    style: &MessageStyle<'_>,
) -> Result<Vec<MessageEvent>, ExtractionError> {
    let request_messages = request_messages(sources);
    let mut events = Vec::with_capacity(request_messages.len());
    for (sequence, message) in request_messages.iter().enumerate() {
        let Json::Object(fields) = message else {
            return Err(ExtractionError::RequestMessageShape);
        };
        events.push(message_event(fields, sequence, style, "user", None));
    }
    Ok(events)
}

fn request_messages<'event>(sources: &'event EventSources<'event>) -> &'event [Json] {
    let payload_messages = sources
        .standard_logging_object
        .and_then(|payload| payload.messages.as_ref());
    match payload_messages {
        Some(Json::List(items)) => items,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => match sources.kwargs.messages {
            Some(Json::List(items)) => items,
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_),
            )
            | None => &[],
        },
    }
}

fn response_events(
    response: &IndexMap<String, Json>,
    style: &MessageStyle<'_>,
    first_sequence: usize,
) -> Result<Vec<MessageEvent>, ExtractionError> {
    let Some(Json::List(choices)) = response.get("choices") else {
        return Ok(Vec::new());
    };
    let mut events = Vec::with_capacity(choices.len());
    let mut sequence = first_sequence;
    for choice in choices {
        let Some(message) = choice_message(choice)? else {
            continue;
        };
        events.push(message_event(
            message,
            sequence,
            style,
            "assistant",
            Some(true),
        ));
        sequence = sequence.saturating_add(1);
    }
    Ok(events)
}

fn choice_message(choice: &Json) -> Result<Option<&IndexMap<String, Json>>, ExtractionError> {
    let Json::Object(fields) = choice else {
        return Err(ExtractionError::ChoiceShape);
    };
    let message = fields.get("message").filter(|value| truthiness(value));
    let delta = fields.get("delta").filter(|value| truthiness(value));
    match message.or(delta) {
        None => Ok(None),
        Some(Json::Object(message_fields)) => Ok(Some(message_fields)),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(ExtractionError::ChoiceShape)
        }
    }
}

fn message_event(
    message: &IndexMap<String, Json>,
    sequence: usize,
    style: &MessageStyle<'_>,
    default_role: &str,
    is_response: Option<bool>,
) -> MessageEvent {
    let role = message_role(message, default_role);
    let content = style.include_content.then(|| message_content(message));
    MessageEvent {
        role,
        sequence,
        response_model: style.response_model.to_owned(),
        vendor: style.vendor.to_owned(),
        is_response,
        content,
        timestamp: style.timestamp,
    }
}

fn message_role(message: &IndexMap<String, Json>, default_role: &str) -> String {
    match message.get("role") {
        Some(Json::Text(role)) if !role.is_empty() => role.clone(),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => default_role.to_owned(),
    }
}
