#![expect(
    clippy::disallowed_types,
    reason = "Responses stream events are provider JSON that betterlite does not model"
)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        reason = "serde_json test fixtures built with json! are concise and type-directed"
    )
)]

mod tests;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_message_scope::fingerprint::{
    stream_item_field, stream_item_fingerprint, stream_item_items,
};
use betterlite_guardrails::guardrail_translation::StreamingScanKey;
use serde_json::Value;

pub const TERMINAL_ENVELOPE_EVENT_TYPES: [&str; 3] = [
    "response.completed",
    "response.failed",
    "response.incomplete",
];
pub const TOOL_CALL_ITEM_TYPES: [&str; 2] = ["function_call", "custom_tool_call"];
pub const TOOL_PAYLOAD_DELTA_EVENT_TYPES: [&str; 2] = [
    "response.function_call_arguments.delta",
    "response.custom_tool_call_input.delta",
];
pub const TOOL_PAYLOAD_DONE_EVENT_FIELDS: [(&str, &str); 2] = [
    ("response.function_call_arguments.done", "arguments"),
    ("response.custom_tool_call_input.done", "input"),
];
pub const TOOL_PAYLOAD_EVENT_TYPES: [&str; 4] = [
    "response.function_call_arguments.delta",
    "response.custom_tool_call_input.delta",
    "response.function_call_arguments.done",
    "response.custom_tool_call_input.done",
];
pub const OUTPUT_ITEM_EVENT_TYPES: [&str; 2] =
    ["response.output_item.added", "response.output_item.done"];
pub const OUTPUT_TEXT_EVENT_TYPES: [&str; 2] =
    ["response.output_text.delta", "response.output_text.done"];

pub(crate) fn integer_field(event: &Value, field: &str) -> Option<usize> {
    event
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

// Nine readers take a buffered event through this one conversion to B3663's typed item.
// ast-grep-ignore: no-padded-one-line-functions, no-one-line-functions
pub(crate) fn event_item(event: &Value) -> Option<StreamItem> {
    let value: GuardrailValue = serde_json::from_value(event.clone()).ok()?;
    Some(StreamItem::Json(value))
}

pub(crate) fn item_text<'item>(item: &'item StreamItem, field: &str) -> Option<&'item str> {
    match stream_item_field(item, field) {
        Some(GuardrailValue::Text(text)) => Some(text),
        Some(_) | None => None,
    }
}

// The scan and both call id lookups ask the same question of an event's item.
// ast-grep-ignore: no-one-line-functions
pub(crate) fn is_tool_call_stream_item(item: &StreamItem) -> bool {
    item_text(item, "type").is_some_and(|kind| TOOL_CALL_ITEM_TYPES.contains(&kind))
}

#[derive(Clone, PartialEq)]
struct PartPosition {
    item_id: Option<GuardrailValue>,
    output_index: Option<GuardrailValue>,
    content_index: Option<GuardrailValue>,
}

#[derive(Default)]
struct PartText {
    done: Option<String>,
    deltas: String,
}

// A missing field and an explicit null are the same part key, as Python's dict.get gives None for both
fn part_position(item: &StreamItem) -> PartPosition {
    PartPosition {
        item_id: stream_item_field(item, "item_id").cloned(),
        output_index: stream_item_field(item, "output_index").cloned(),
        content_index: stream_item_field(item, "content_index").cloned(),
    }
}

fn accumulate_part(parts: &mut Vec<(PartPosition, PartText)>, item: &StreamItem) {
    let text = item_text(item, "text");
    let delta = item_text(item, "delta");
    if text.is_none() && delta.is_none() {
        return;
    }
    let position = part_position(item);
    let index = match parts.iter().position(|(existing, _)| *existing == position) {
        Some(index) => index,
        None => {
            let index = parts.len();
            parts.push((position, PartText::default()));
            index
        }
    };
    let Some((_, part)) = parts.get_mut(index) else {
        return;
    };
    if let Some(text) = text {
        part.done = Some(text.to_owned());
    }
    if let Some(delta) = delta {
        part.deltas.push_str(delta);
    }
}

#[must_use]
pub fn streaming_string_so_far(events: &[Value]) -> String {
    let mut parts: Vec<(PartPosition, PartText)> = Vec::new();
    for event in events {
        let Some(item) = event_item(event) else {
            continue;
        };
        accumulate_part(&mut parts, &item);
    }
    let mut joined = String::new();
    for (_, part) in &parts {
        match &part.done {
            Some(text) => joined.push_str(text),
            None => joined.push_str(&part.deltas),
        }
    }
    joined
}

#[must_use]
pub fn streaming_has_ended(events: &[Value]) -> bool {
    let Some(last) = events.last() else {
        return false;
    };
    let Some(item) = event_item(last) else {
        return false;
    };
    item_text(&item, "type")
        .is_some_and(|event_type| TERMINAL_ENVELOPE_EVENT_TYPES.contains(&event_type))
}

#[must_use]
pub fn has_streamed_tool_call_events(events: &[Value]) -> bool {
    for event in events {
        let Some(item) = event_item(event) else {
            continue;
        };
        let Some(event_type) = item_text(&item, "type") else {
            continue;
        };
        if TOOL_PAYLOAD_EVENT_TYPES.contains(&event_type) {
            return true;
        }
        if !OUTPUT_ITEM_EVENT_TYPES.contains(&event_type) {
            continue;
        }
        let Some(inner) = stream_item_field(&item, "item")
            .cloned()
            .map(StreamItem::Json)
        else {
            continue;
        };
        if is_tool_call_stream_item(&inner) {
            return true;
        }
    }
    false
}

#[must_use]
pub fn completed_response_scan_key(response: Option<&Value>) -> StreamingScanKey {
    let envelope = response.and_then(event_item);
    let empty: &[GuardrailValue] = &[];
    let items = envelope
        .as_ref()
        .map_or(empty, |item| stream_item_items(item, "output"));
    let mut texts = Vec::new();
    let mut tool_calls = Vec::new();
    for item in items {
        let stream_item = StreamItem::Json(item.clone());
        if is_tool_call_stream_item(&stream_item) {
            if let Ok(fingerprint) = stream_item_fingerprint(&stream_item) {
                tool_calls.push(fingerprint);
            }
            continue;
        }
        for part in stream_item_items(&stream_item, "content") {
            let content_part = StreamItem::Json(part.clone());
            if let Some(text) = item_text(&content_part, "text").filter(|text| !text.is_empty()) {
                texts.push(text.to_owned());
            }
        }
    }
    StreamingScanKey {
        texts,
        tool_calls,
        stream_ended: true,
        tool_calls_in_flight: false,
    }
}

#[must_use]
pub fn streaming_scan_key(events: &[Value]) -> Option<StreamingScanKey> {
    let last = events.last()?;
    if !last.is_object() {
        return None;
    }
    let item = event_item(last)?;
    let last_type = item_text(&item, "type");
    if last_type == Some("response.output_item.done") {
        return None;
    }
    if last_type.is_some_and(|event_type| TERMINAL_ENVELOPE_EVENT_TYPES.contains(&event_type)) {
        let response =
            stream_item_field(&item, "response").and_then(|value| serde_json::to_value(value).ok());
        return Some(completed_response_scan_key(response.as_ref()));
    }
    Some(StreamingScanKey {
        texts: vec![streaming_string_so_far(events)],
        tool_calls: Vec::new(),
        stream_ended: false,
        tool_calls_in_flight: has_streamed_tool_call_events(events),
    })
}
