use std::borrow::Cow;
use std::collections::BTreeMap;
use std::str::from_utf8;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::de::{DeserializeOwned, Error as _};
use serde_json::value::RawValue;

use super::error::EventError;
use crate::realtime_completion::tool_input::{ToolUse, ToolUseEvent};
use crate::realtime_output::events::{AudioOutput, ContentStart, TextOutput};
use crate::realtime_session::records::UsageEvent;

// LiteLLM keeps message[:200] of a message that is not JSON, counted in characters of the text it was given
const PREVIEW_CHARS: usize = 200;

// A provider frame or client message past this size never reaches the parser, so its size cannot size the parse
pub(super) const MAX_MESSAGE_BYTES: usize = 0x0010_0000;

type EventFields<'event> = BTreeMap<Cow<'event, str>, &'event RawValue>;

#[derive(Debug)]
pub enum NativeEvent {
    SessionStart,
    ContentStart(Box<ContentStart>),
    TextOutput(TextOutput),
    AudioOutput(AudioOutput),
    // The stop reason is read only when no user transcript is active, so the payload stays raw here
    ContentEnd(String),
    UserSpeechStart,
    UserSpeechEnd,
    UsageEvent(UsageEvent),
    ToolUse(Box<ToolUseEvent>),
    PromptEnd,
    Unknown,
}

#[derive(Debug)]
pub enum DecodedEvent {
    Event(NativeEvent),
    InvalidJson { preview: String },
}

#[derive(Clone, Copy)]
enum Branch<'event> {
    SessionStart,
    ContentStart(&'event RawValue),
    TextOutput(&'event RawValue),
    AudioOutput(&'event RawValue),
    ContentEnd(&'event RawValue),
    UserSpeechStart,
    UserSpeechEnd,
    UsageEvent(&'event RawValue),
    ToolUse(&'event RawValue),
    PromptEnd,
}

// A syntax error is the one parse failure LiteLLM answers with a warning, so it comes back as a decoded value
pub fn decode_realtime_native_event(bytes: &[u8]) -> Result<DecodedEvent, EventError> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(EventError::ExceedsLimit {
            subject: "provider event",
            limit: MAX_MESSAGE_BYTES,
        });
    }
    let text = from_utf8(bytes).map_err(|source| EventError::Utf8 { source })?;
    // The whole message must parse before its shape is judged, so an unterminated array is a syntax warning
    let syntax: Result<&RawValue, serde_json::Error> = serde_json::from_str(text);
    if syntax.is_err() {
        let preview = text.chars().take(PREVIEW_CHARS).collect();
        return Ok(DecodedEvent::InvalidJson { preview });
    }
    let fields: EventFields<'_> =
        serde_json::from_str(text).map_err(|source| EventError::NotAnObject { source })?;
    let Some(event) = fields.get("event").copied() else {
        return Ok(DecodedEvent::Event(NativeEvent::Unknown));
    };
    let event_fields: EventFields<'_> = serde_json::from_str(event.get())
        .map_err(|source| EventError::EventNotAnObject { source })?;
    Ok(DecodedEvent::Event(
        branch(&event_fields).map_or(Ok(NativeEvent::Unknown), payload)?,
    ))
}

// _parse_client_message plus _json_dict: JSON that is not an object reads as the empty mapping, never an error
#[must_use]
pub fn decode_realtime_client_preface(bytes: &[u8]) -> IndexMap<String, Json> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return IndexMap::new();
    }
    let Ok(Json::Object(fields)) = serde_json::from_slice::<Json>(bytes) else {
        return IndexMap::new();
    };
    fields
}

#[must_use]
pub const fn json_dict(value: &Json) -> Option<&IndexMap<String, Json>> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

#[must_use]
pub fn json_str(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// The branch order is LiteLLM's if/elif chain: the first key present decides, and every later key is ignored
fn branch<'event>(event: &EventFields<'event>) -> Option<Branch<'event>> {
    if event.contains_key("sessionStart") {
        return Some(Branch::SessionStart);
    }
    if let Some(raw) = event.get("contentStart").copied() {
        return Some(Branch::ContentStart(raw));
    }
    if let Some(raw) = event.get("textOutput").copied() {
        return Some(Branch::TextOutput(raw));
    }
    if let Some(raw) = event.get("audioOutput").copied() {
        return Some(Branch::AudioOutput(raw));
    }
    if let Some(raw) = event.get("contentEnd").copied() {
        return Some(Branch::ContentEnd(raw));
    }
    if event.contains_key("userSpeechStart") {
        return Some(Branch::UserSpeechStart);
    }
    if event.contains_key("userSpeechEnd") {
        return Some(Branch::UserSpeechEnd);
    }
    if let Some(raw) = event.get("usageEvent").copied() {
        return Some(Branch::UsageEvent(raw));
    }
    if let Some(raw) = event.get("toolUse").copied() {
        return Some(Branch::ToolUse(raw));
    }
    if event.contains_key("promptEnd") || event.contains_key("completionEnd") {
        return Some(Branch::PromptEnd);
    }
    None
}

// LiteLLM validates usageEvent as a pydantic model, so each nested record is an object; serde would take an array positionally
fn require_usage_objects(text: &str) -> Result<(), EventError> {
    let event: Json = serde_json::from_str(text).map_err(|source| EventError::Payload {
        field: "usageEvent",
        source,
    })?;
    let details = json_dict(&event).and_then(|fields| fields.get("details"));
    let total = details
        .and_then(json_dict)
        .and_then(|fields| fields.get("total"));
    let input = total
        .and_then(json_dict)
        .and_then(|fields| fields.get("input"));
    let output = total
        .and_then(json_dict)
        .and_then(|fields| fields.get("output"));
    for record in [details, total, input, output].into_iter().flatten() {
        if !matches!(record, Json::Object(_)) {
            return Err(EventError::Payload {
                field: "usageEvent",
                source: serde_json::Error::custom("the usage record is not a JSON object"),
            });
        }
    }
    Ok(())
}

// A payload is always a JSON object; serde would read an array into the declared struct as positional fields
pub(super) fn require_object(text: &str, field: &'static str) -> Result<(), EventError> {
    if text.trim_start().starts_with('{') {
        return Ok(());
    }
    Err(EventError::Payload {
        field,
        source: serde_json::Error::custom("the event payload is not a JSON object"),
    })
}

fn payload(branch: Branch<'_>) -> Result<NativeEvent, EventError> {
    match branch {
        Branch::SessionStart => Ok(NativeEvent::SessionStart),
        Branch::ContentStart(raw) => decode_object(raw, "contentStart")
            .map(|event| NativeEvent::ContentStart(Box::new(event))),
        Branch::TextOutput(raw) => decode_object(raw, "textOutput").map(NativeEvent::TextOutput),
        Branch::AudioOutput(raw) => decode_object(raw, "audioOutput").map(NativeEvent::AudioOutput),
        Branch::ContentEnd(raw) => Ok(NativeEvent::ContentEnd(raw.get().to_owned())),
        Branch::UserSpeechStart => Ok(NativeEvent::UserSpeechStart),
        Branch::UserSpeechEnd => Ok(NativeEvent::UserSpeechEnd),
        Branch::UsageEvent(raw) => usage_event(raw),
        Branch::ToolUse(raw) => decode_object(raw, "toolUse")
            .map(|tool_use: ToolUse| NativeEvent::ToolUse(Box::new(ToolUseEvent { tool_use }))),
        Branch::PromptEnd => Ok(NativeEvent::PromptEnd),
    }
}

fn decode_object<T>(raw: &RawValue, field: &'static str) -> Result<T, EventError>
where
    T: DeserializeOwned,
{
    require_object(raw.get(), field)?;
    serde_json::from_str(raw.get()).map_err(|source| EventError::Payload { field, source })
}

fn usage_event(raw: &RawValue) -> Result<NativeEvent, EventError> {
    require_object(raw.get(), "usageEvent")?;
    require_usage_objects(raw.get())?;
    serde_json::from_str(raw.get())
        .map(NativeEvent::UsageEvent)
        .map_err(|source| EventError::Payload {
            field: "usageEvent",
            source,
        })
}
