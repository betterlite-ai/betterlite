use std::borrow::Cow;
use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use serde_json::error::Category;
use serde_json::value::RawValue;

use super::error::EventError;
use crate::realtime_completion::tool_input::{ToolUse, ToolUseEvent};
use crate::realtime_output::events::{AudioOutput, ContentStart, TextOutput};
use crate::realtime_session::records::UsageEvent;

// LiteLLM keeps message[:200] of a message that is not JSON, counted in characters of the text it was given
const PREVIEW_CHARS: usize = 200;

type EventFields<'a> = BTreeMap<Cow<'a, str>, &'a RawValue>;

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
enum Branch<'a> {
    SessionStart,
    ContentStart(&'a RawValue),
    TextOutput(&'a RawValue),
    AudioOutput(&'a RawValue),
    ContentEnd(&'a RawValue),
    UserSpeechStart,
    UserSpeechEnd,
    UsageEvent(&'a RawValue),
    ToolUse(&'a RawValue),
    PromptEnd,
}

// A syntax error is the one parse failure LiteLLM answers with a warning, so it comes back as a decoded value
pub fn decode_realtime_native_event(bytes: &[u8]) -> Result<DecodedEvent, EventError> {
    let text = std::str::from_utf8(bytes).map_err(|source| EventError::Utf8 { source })?;
    let parsed: Result<EventFields<'_>, serde_json::Error> = serde_json::from_str(text);
    let fields = match parsed {
        Ok(fields) => fields,
        Err(source) if matches!(source.classify(), Category::Data) => {
            return Err(EventError::NotAnObject { source });
        }
        Err(_) => {
            let preview = text.chars().take(PREVIEW_CHARS).collect();
            return Ok(DecodedEvent::InvalidJson { preview });
        }
    };
    let Some(event) = fields.get("event").copied() else {
        return Ok(DecodedEvent::Event(NativeEvent::Unknown));
    };
    let parsed: Result<EventFields<'_>, serde_json::Error> = serde_json::from_str(event.get());
    let event_fields = parsed.map_err(|source| EventError::EventNotAnObject { source })?;
    Ok(DecodedEvent::Event(classify(&event_fields)?))
}

// _parse_client_message plus _json_dict: JSON that is not an object reads as the empty mapping, never an error
#[must_use]
pub fn decode_realtime_client_preface(bytes: &[u8]) -> IndexMap<String, Json> {
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
pub const fn json_str(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn classify(event: &EventFields<'_>) -> Result<NativeEvent, EventError> {
    match branch(event) {
        Some(branch) => payload(branch),
        None => Ok(NativeEvent::Unknown),
    }
}

// The branch order is LiteLLM's if/elif chain: the first key present decides, and every later key is ignored
fn branch<'a>(event: &EventFields<'a>) -> Option<Branch<'a>> {
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
    speech_or_end(event)
}

fn speech_or_end<'a>(event: &EventFields<'a>) -> Option<Branch<'a>> {
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

fn payload(branch: Branch<'_>) -> Result<NativeEvent, EventError> {
    match branch {
        Branch::SessionStart => Ok(NativeEvent::SessionStart),
        Branch::ContentStart(raw) => {
            decode(raw, "contentStart").map(|event| NativeEvent::ContentStart(Box::new(event)))
        }
        Branch::TextOutput(raw) => decode(raw, "textOutput").map(NativeEvent::TextOutput),
        Branch::AudioOutput(raw) => decode(raw, "audioOutput").map(NativeEvent::AudioOutput),
        Branch::ContentEnd(raw) => Ok(NativeEvent::ContentEnd(raw.get().to_owned())),
        Branch::UserSpeechStart => Ok(NativeEvent::UserSpeechStart),
        Branch::UserSpeechEnd => Ok(NativeEvent::UserSpeechEnd),
        Branch::UsageEvent(raw) => decode(raw, "usageEvent").map(NativeEvent::UsageEvent),
        Branch::ToolUse(raw) => decode(raw, "toolUse")
            .map(|tool_use: ToolUse| NativeEvent::ToolUse(Box::new(ToolUseEvent { tool_use }))),
        Branch::PromptEnd => Ok(NativeEvent::PromptEnd),
    }
}

fn decode<T: DeserializeOwned>(raw: &RawValue, field: &'static str) -> Result<T, EventError> {
    serde_json::from_str(raw.get()).map_err(|source| EventError::Payload { field, source })
}
