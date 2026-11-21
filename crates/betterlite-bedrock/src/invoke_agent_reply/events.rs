use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

use crate::event_frames::AwsEvent;
use crate::event_frames::message::parse_message_from_event;
use crate::event_frames::shape::ResponseStreamShape;

const EVENT_TYPE: &str = ":event-type";
const CONTENT_TYPE: &str = ":content-type";
const MESSAGE_TYPE: &str = ":message-type";
const CHUNK: &str = "chunk";
const TRACE: &str = "trace";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventHeaders {
    pub event_type: String,
    pub content_type: String,
    pub message_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEvent {
    pub headers: EventHeaders,
    pub payload: Json,
}

#[derive(Debug, Error)]
pub enum HeaderTextError {
    #[error("event-stream header {name} is not a string")]
    NotText { name: &'static str },
}

// _extract_headers_from_event: the three categorization headers, an absent one reading as the empty string.
pub fn extract_headers(frame: &AwsEvent) -> Result<EventHeaders, HeaderTextError> {
    Ok(EventHeaders {
        event_type: header_text(frame, EVENT_TYPE)?,
        content_type: header_text(frame, CONTENT_TYPE)?,
        message_type: header_text(frame, MESSAGE_TYPE)?,
    })
}

// One recognized frame as LiteLLM records it: a chunk keeps its text re-encoded as base64, a trace its parsed JSON.
pub fn decode_frame(
    shape: &ResponseStreamShape,
    frame: &AwsEvent,
    headers: &EventHeaders,
) -> Option<AgentEvent> {
    let payload = match headers.event_type.as_str() {
        CHUNK => {
            let text = message_text(shape, frame)?;
            Json::Object(IndexMap::from([(
                "bytes".to_owned(),
                Json::Text(STANDARD.encode(text.as_bytes())),
            )]))
        }
        TRACE => trace_payload(&message_text(shape, frame)?)?,
        _ => {
            tracing::debug!("Unknown event type: {}", headers.event_type);
            return None;
        }
    };
    Some(AgentEvent {
        headers: headers.clone(),
        payload,
    })
}

fn header_text(frame: &AwsEvent, name: &'static str) -> Result<String, HeaderTextError> {
    let Some(value) = frame.headers.get(name) else {
        return Ok(String::new());
    };
    value
        .as_string()
        .map(|text| text.as_str().to_owned())
        .map_err(|_| HeaderTextError::NotText { name })
}

// _parse_message_from_event returns None for an empty or failed message, and the frame's inner catch swallows every parser error.
fn message_text(shape: &ResponseStreamShape, frame: &AwsEvent) -> Option<String> {
    match parse_message_from_event(Some(shape), frame) {
        Ok(Some(text)) if !text.is_empty() => Some(text),
        Ok(_) => None,
        Err(error) => {
            tracing::debug!("Error parsing message from event: {error}");
            None
        }
    }
}

fn trace_payload(text: &str) -> Option<Json> {
    match serde_json::from_str::<Json>(text) {
        Ok(payload) => Some(payload),
        Err(error) => {
            tracing::warn!("Failed to parse trace event JSON: {error}");
            None
        }
    }
}
