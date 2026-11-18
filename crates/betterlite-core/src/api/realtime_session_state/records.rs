use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::SessionStateError;
use crate::api::realtime_contracts::decode::{RealtimeDecodeLimits, decode_realtime_frame};

// LiteLLM decodes a logged frame with json.loads, which accepts a frame of any size; the cap keeps a malformed relay frame out of the log
pub(crate) const LOGGING_FRAME_LIMITS: RealtimeDecodeLimits =
    RealtimeDecodeLimits { max_bytes: 1 << 24 };

// LiteLLM builds OpenAIRealtimeStreamSessionEvents for these two types and the catch-all base object for every other name
const SESSION_EVENT_TYPES: [&str; 2] = ["session.created", "session.updated"];

#[derive(Debug, thiserror::Error)]
#[error("the {event_type} event carries no session object")]
pub struct SessionEventShapeError {
    pub event_type: String,
}

pub(crate) fn event_type(record: &IndexMap<String, Json>) -> Option<&str> {
    match record.get("type") {
        Some(Json::Text(text)) if !text.is_empty() => Some(text.as_str()),
        _ => None,
    }
}

pub(crate) fn decoded_record(bytes: &[u8]) -> Result<IndexMap<String, Json>, SessionStateError> {
    match decode_realtime_frame(bytes, LOGGING_FRAME_LIMITS)? {
        Json::Object(record) => Ok(record),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(SessionStateError::NotAnEvent)
        }
    }
}

// LiteLLM's TypedDict construction keeps every key of the mapping, so a session event only has to carry a session object
pub(crate) fn check_session_event(
    record: &IndexMap<String, Json>,
) -> Result<(), SessionEventShapeError> {
    let Some(event_type) = event_type(record) else {
        return Ok(());
    };
    if !SESSION_EVENT_TYPES.contains(&event_type) {
        return Ok(());
    }
    match record.get("session") {
        Some(Json::Object(_)) => Ok(()),
        _ => Err(SessionEventShapeError {
            event_type: event_type.to_owned(),
        }),
    }
}

pub(crate) fn tool_call_record(item: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let mut function = IndexMap::new();
    function.insert(
        "name".to_owned(),
        item.get("name")
            .cloned()
            .unwrap_or_else(|| Json::Text(String::new())),
    );
    function.insert(
        "arguments".to_owned(),
        item.get("arguments")
            .cloned()
            .unwrap_or_else(|| Json::Text("{}".to_owned())),
    );
    let mut record = IndexMap::new();
    record.insert(
        "id".to_owned(),
        item.get("call_id")
            .cloned()
            .unwrap_or_else(|| Json::Text(String::new())),
    );
    record.insert("type".to_owned(), Json::Text("function".to_owned()));
    record.insert("function".to_owned(), Json::Object(function));
    record
}
