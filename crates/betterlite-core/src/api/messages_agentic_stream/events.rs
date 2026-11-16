use std::collections::{BTreeMap, BTreeSet};
use std::str::from_utf8;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::{MessagesAgenticError, decode_messages_agentic_stream};
use crate::api::api_boundaries::ApiLimits;

mod blocks;
mod response;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event_type: String,
    pub data: Json,
}

// LiteLLM's _parse_sse_events: event: sets the pending type, data: decodes one payload
pub fn parse_sse_events(
    raw: &[u8],
    limits: &ApiLimits,
) -> Result<Vec<SseEvent>, MessagesAgenticError> {
    let text = from_utf8(raw).map_err(MessagesAgenticError::Utf8)?;
    let mut events = Vec::new();
    let mut current_event_type: Option<String> = None;
    for line in text.split('\n') {
        let stripped = line.trim();
        if let Some(event_name) = stripped.strip_prefix("event:") {
            current_event_type = Some(event_name.trim().to_owned());
            continue;
        }
        let Some(data_str) = stripped.strip_prefix("data:") else {
            continue;
        };
        let Ok(data) = decode_messages_agentic_stream(data_str.trim().as_bytes(), limits) else {
            continue;
        };
        // a decode failure keeps the pending event: name, exactly as LiteLLM's continue does
        let event_type = event_kind(current_event_type.take(), &data);
        events.push(SseEvent { event_type, data });
    }
    Ok(events)
}

fn event_kind(pending: Option<String>, data: &Json) -> String {
    match pending {
        Some(kind) if !kind.is_empty() => kind,
        _ => match data {
            Json::Object(fields) => text_field(fields, "type").unwrap_or_default().to_owned(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                String::new()
            }
        },
    }
}

// LiteLLM's _rebuild_anthropic_response_from_sse: None when no message_start arrived
pub fn rebuild_agentic_response(
    raw: &[u8],
    limits: &ApiLimits,
) -> Result<Option<Json>, MessagesAgenticError> {
    let events = parse_sse_events(raw, limits)?;
    let mut content_blocks: BTreeMap<usize, IndexMap<String, Json>> = BTreeMap::new();
    let mut response = envelope();
    let mut saw_message_start = false;
    for event in &events {
        match event.event_type.as_str() {
            "message_start" => {
                saw_message_start = true;
                response::handle_message_start(&event.data, &mut response)?;
            }
            "content_block_start" => {
                blocks::handle_content_block_start(&event.data, &mut content_blocks)?;
            }
            "content_block_delta" => {
                blocks::handle_content_block_delta(&event.data, &mut content_blocks)?;
            }
            "content_block_stop" => {
                blocks::handle_content_block_stop(&event.data, &mut content_blocks, limits)?;
            }
            "message_delta" => response::handle_message_delta(&event.data, &mut response)?,
            _other => {}
        }
    }
    if !saw_message_start {
        return Ok(None);
    }
    let content = content_blocks
        .into_values()
        .map(finalize_block)
        .collect::<Vec<_>>();
    response.insert("content".to_owned(), Json::List(content));
    Ok(Some(Json::Object(response)))
}

fn envelope() -> IndexMap<String, Json> {
    let mut usage = IndexMap::new();
    usage.insert("input_tokens".to_owned(), Json::Number(Number::from(0_u8)));
    usage.insert("output_tokens".to_owned(), Json::Number(Number::from(0_u8)));
    let mut response = IndexMap::new();
    response.insert("id".to_owned(), Json::Text(String::new()));
    response.insert("type".to_owned(), Json::Text("message".to_owned()));
    response.insert("role".to_owned(), Json::Text("assistant".to_owned()));
    response.insert("model".to_owned(), Json::Text(String::new()));
    response.insert("content".to_owned(), Json::List(Vec::new()));
    response.insert("stop_reason".to_owned(), Json::Null);
    response.insert("stop_sequence".to_owned(), Json::Null);
    response.insert("usage".to_owned(), Json::Object(usage));
    response
}

fn finalize_block(mut block: IndexMap<String, Json>) -> Json {
    block.shift_remove("_partial_json");
    Json::Object(block)
}

// LiteLLM's _buffer_holds_server_fulfilled_tool_use, read from the content_block_start events
pub fn buffer_holds_server_tool_use(
    raw: &[u8],
    names: &BTreeSet<String>,
    limits: &ApiLimits,
) -> Result<bool, MessagesAgenticError> {
    if names.is_empty() {
        return Ok(false);
    }
    let events = parse_sse_events(raw, limits)?;
    Ok(events.iter().any(|event| {
        event.event_type == "content_block_start"
            && match &event.data {
                Json::Object(fields) => started_tool_use(fields, names),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    false
                }
            }
    }))
}

fn started_tool_use(fields: &IndexMap<String, Json>, names: &BTreeSet<String>) -> bool {
    match fields.get("content_block") {
        Some(Json::Object(block)) => {
            text_field(block, "type") == Some("tool_use")
                && text_field(block, "name").is_some_and(|name| names.contains(name))
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => false,
    }
}

pub(super) fn text_field<'fields>(
    fields: &'fields IndexMap<String, Json>,
    key: &str,
) -> Option<&'fields str> {
    if let Some(Json::Text(text)) = fields.get(key) {
        return Some(text);
    }
    None
}

pub(super) fn event_fields(data: &Json) -> Result<&IndexMap<String, Json>, MessagesAgenticError> {
    if let Json::Object(fields) = data {
        return Ok(fields);
    }
    Err(shape_error("data", "an object"))
}

pub(super) fn object_field<'value>(
    fields: &'value IndexMap<String, Json>,
    key: &str,
) -> Result<Option<&'value IndexMap<String, Json>>, MessagesAgenticError> {
    match fields.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(object)) => Ok(Some(object)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(shape_error(key, "an object"))
        }
    }
}

pub(super) fn index_field(
    fields: &IndexMap<String, Json>,
) -> Result<Option<usize>, MessagesAgenticError> {
    let Some(value) = fields.get("index") else {
        return Ok(None);
    };
    if matches!(value, Json::Null) {
        return Ok(None);
    }
    let Json::Number(number) = value else {
        return Err(shape_error("index", "a non-negative integer"));
    };
    number
        .as_u64()
        .and_then(|index| usize::try_from(index).ok())
        .map(Some)
        .ok_or_else(|| shape_error("index", "a non-negative integer"))
}

pub(super) fn shape_error(field: &str, expected: &'static str) -> MessagesAgenticError {
    MessagesAgenticError::Shape {
        field: field.to_owned(),
        expected,
    }
}
