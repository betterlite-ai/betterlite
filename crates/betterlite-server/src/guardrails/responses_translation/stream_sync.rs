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

pub mod error;
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_message_scope::fingerprint::stream_item_field;
use serde_json::Value;

use crate::guardrails::responses_translation::input_provenance::is_tool_call_output_item;
use crate::guardrails::responses_translation::output_rewrite::write_tool_call_item;
use crate::guardrails::responses_translation::stream_scan::{
    OUTPUT_ITEM_EVENT_TYPES, OUTPUT_TEXT_EVENT_TYPES, TOOL_CALL_ITEM_TYPES,
    TOOL_PAYLOAD_DELTA_EVENT_TYPES, TOOL_PAYLOAD_DONE_EVENT_FIELDS, TOOL_PAYLOAD_EVENT_TYPES,
    event_item, integer_field, item_text,
};
use crate::guardrails::tool_call_identity::error::UndeliverableRewriteReason;
use crate::guardrails::tool_call_identity::{
    ToolCallShape, UndeliverableRewriteInput, tool_call_rewrite, undeliverable_reason,
};
use error::StreamSyncError;

const SPREAD_REASON: &str = "the scanned text events are not all output_text deltas with an integer output_index and content_index, so the text rewrite has nowhere to land";

#[derive(Clone, PartialEq)]
struct TextPosition {
    output_index: Option<GuardrailValue>,
    content_index: Option<GuardrailValue>,
}

fn text_position(item: &StreamItem) -> TextPosition {
    TextPosition {
        output_index: stream_item_field(item, "output_index").cloned(),
        content_index: stream_item_field(item, "content_index").cloned(),
    }
}

fn unsigned(value: Option<&GuardrailValue>) -> Option<usize> {
    match value {
        Some(GuardrailValue::SignedInteger(number)) => usize::try_from(*number).ok(),
        Some(GuardrailValue::UnsignedInteger(number)) => usize::try_from(*number).ok(),
        Some(_) | None => None,
    }
}

fn spread_undeliverable(guardrail_name: &str) -> StreamSyncError {
    StreamSyncError::Undeliverable {
        guardrail_name: guardrail_name.to_owned(),
        reason: SPREAD_REASON.to_owned(),
    }
}

// ast-grep-ignore: no-long-names
pub fn sync_stream_events_with_rewrites(
    events: &mut [Value],
    rewrites: &BTreeMap<(usize, usize), String>,
) {
    let mut delivered: BTreeSet<(usize, usize)> = BTreeSet::new();
    for event in events.iter_mut() {
        sync_event(event, rewrites, &mut delivered);
    }
}

fn sync_event(
    event: &mut Value,
    rewrites: &BTreeMap<(usize, usize), String>,
    delivered: &mut BTreeSet<(usize, usize)>,
) {
    let Some(output_index) = integer_field(event, "output_index") else {
        return;
    };
    if event.get("type").and_then(Value::as_str) == Some("response.output_item.done") {
        if let Some(item) = event.get_mut("item") {
            sync_output_item_done_event(item, output_index, rewrites);
        }
        return;
    }
    let Some(content_index) = integer_field(event, "content_index") else {
        return;
    };
    let position = (output_index, content_index);
    if rewrites.contains_key(&position) {
        sync_text_position(event, position, rewrites, delivered);
    }
}

fn sync_text_position(
    event: &mut Value,
    position: (usize, usize),
    rewrites: &BTreeMap<(usize, usize), String>,
    delivered: &mut BTreeSet<(usize, usize)>,
) {
    let Some(fields) = event.as_object_mut() else {
        return;
    };
    let Some(event_type) = fields
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return;
    };
    let Some(rewritten) = rewrites.get(&position) else {
        return;
    };
    match event_type.as_str() {
        "response.output_text.delta" => {
            let value = if delivered.insert(position) {
                rewritten.clone()
            } else {
                String::new()
            };
            fields.insert("delta".to_owned(), Value::String(value));
        }
        "response.output_text.done" => {
            fields.insert("text".to_owned(), Value::String(rewritten.clone()));
        }
        "response.content_part.done" => {
            if let Some(part) = fields.get_mut("part").and_then(Value::as_object_mut) {
                part.insert("text".to_owned(), Value::String(rewritten.clone()));
            }
        }
        _ => {}
    }
}

pub fn sync_output_item_done_event(
    item: &mut Value,
    output_index: usize,
    rewrites: &BTreeMap<(usize, usize), String>,
) {
    let Some(content) = item.get_mut("content").and_then(Value::as_array_mut) else {
        return;
    };
    for ((item_index, content_index), rewritten) in rewrites {
        if *item_index != output_index {
            continue;
        }
        let Some(entry) = content
            .get_mut(*content_index)
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        entry.insert("text".to_owned(), Value::String(rewritten.clone()));
    }
}

fn scanned_text_positions(
    events: &[Value],
    guardrail_name: &str,
) -> Result<Vec<TextPosition>, StreamSyncError> {
    let mut positions: Vec<TextPosition> = Vec::new();
    for event in events {
        let Some(item) = event_item(event) else {
            continue;
        };
        if item_text(&item, "text").is_none() && item_text(&item, "delta").is_none() {
            continue;
        }
        let event_type = item_text(&item, "type");
        if !event_type.is_some_and(|kind| OUTPUT_TEXT_EVENT_TYPES.contains(&kind)) {
            return Err(spread_undeliverable(guardrail_name));
        }
        let position = text_position(&item);
        if !positions.iter().any(|existing| *existing == position) {
            positions.push(position);
        }
    }
    Ok(positions)
}

fn placeable_positions(
    positions: &[TextPosition],
    guardrail_name: &str,
) -> Result<Vec<(usize, usize)>, StreamSyncError> {
    let mut placeable = Vec::with_capacity(positions.len());
    for position in positions {
        let output_index = unsigned(position.output_index.as_ref());
        let content_index = unsigned(position.content_index.as_ref());
        let (Some(output_index), Some(content_index)) = (output_index, content_index) else {
            return Err(spread_undeliverable(guardrail_name));
        };
        placeable.push((output_index, content_index));
    }
    Ok(placeable)
}

// ast-grep-ignore: no-long-names
pub fn spread_text_rewrite_over_stream_events(
    events: &mut [Value],
    rewritten_text: &str,
    guardrail_name: &str,
) -> Result<(), StreamSyncError> {
    let scanned = scanned_text_positions(events, guardrail_name)?;
    let positions = placeable_positions(&scanned, guardrail_name)?;
    let mut rewrites = BTreeMap::new();
    for (index, position) in positions.into_iter().enumerate() {
        let text = if index == 0 {
            rewritten_text.to_owned()
        } else {
            String::new()
        };
        rewrites.insert(position, text);
    }
    sync_stream_events_with_rewrites(events, &rewrites);
    Ok(())
}

#[must_use]
pub fn tool_call_ids_by_item_id(events: &[Value]) -> BTreeMap<String, String> {
    let mut ids = BTreeMap::new();
    for event in events {
        let Some(item) = event_item(event) else {
            continue;
        };
        let is_output_item_event = item_text(&item, "type")
            .is_some_and(|event_type| OUTPUT_ITEM_EVENT_TYPES.contains(&event_type));
        if !is_output_item_event {
            continue;
        }
        let Some(inner) = stream_item_field(&item, "item") else {
            continue;
        };
        let inner = StreamItem::Json(inner.clone());
        let is_tool_call_item = item_text(&inner, "type")
            .is_some_and(|inner_type| TOOL_CALL_ITEM_TYPES.contains(&inner_type));
        if !is_tool_call_item {
            continue;
        }
        let (Some(item_id), Some(call_id)) =
            (item_text(&inner, "id"), item_text(&inner, "call_id"))
        else {
            continue;
        };
        ids.insert(item_id.to_owned(), call_id.to_owned());
    }
    ids
}

#[must_use]
pub fn tool_call_event_call_id(
    event: &Value,
    call_id_by_item_id: &BTreeMap<String, String>,
) -> Option<String> {
    let item = event_item(event)?;
    let event_type = item_text(&item, "type")?;
    if TOOL_PAYLOAD_EVENT_TYPES.contains(&event_type) {
        let item_id = item_text(&item, "item_id")?;
        return call_id_by_item_id.get(item_id).cloned();
    }
    if !OUTPUT_ITEM_EVENT_TYPES.contains(&event_type) {
        return None;
    }
    let inner = StreamItem::Json(stream_item_field(&item, "item")?.clone());
    let is_tool_call_item = item_text(&inner, "type")
        .is_some_and(|inner_type| TOOL_CALL_ITEM_TYPES.contains(&inner_type));
    if !is_tool_call_item {
        return None;
    }
    item_text(&inner, "call_id").map(str::to_owned)
}

fn is_payload_event(event: &Value) -> bool {
    let Some(item) = event_item(event) else {
        return false;
    };
    item_text(&item, "type")
        .is_some_and(|event_type| TOOL_PAYLOAD_EVENT_TYPES.contains(&event_type))
}

fn tool_call_item_count(output: &[Value]) -> usize {
    output
        .iter()
        .filter(|item| is_tool_call_output_item(item))
        .count()
}

fn tool_call_call_ids(output: &[Value]) -> Vec<String> {
    let mut call_ids = Vec::new();
    for item in output {
        if !is_tool_call_output_item(item) {
            continue;
        }
        let Some(call_id) = item.get("call_id").and_then(Value::as_str) else {
            continue;
        };
        if call_id.is_empty() {
            continue;
        }
        call_ids.push(call_id.to_owned());
    }
    call_ids
}

fn tool_call_rewrite_map(
    call_ids: &[String],
    pre: &[ToolCallShape],
    post: &[ToolCallShape],
) -> BTreeMap<String, ToolCallShape> {
    let mut rewrites = BTreeMap::new();
    for ((call_id, before), after) in call_ids.iter().zip(pre).zip(post) {
        if before == after {
            continue;
        }
        rewrites.insert(call_id.clone(), tool_call_rewrite(before, after));
    }
    rewrites
}

fn undeliverable_tool_call_rewrite(
    earlier: &[Value],
    call_ids: &[String],
    tool_call_items: usize,
    post: &[ToolCallShape],
    event_call_ids: &[Option<String>],
    rewrites: &BTreeMap<String, ToolCallShape>,
) -> Option<UndeliverableRewriteReason> {
    let unresolved_argument_event = earlier
        .iter()
        .zip(event_call_ids)
        .any(|(event, call_id)| call_id.is_none() && is_payload_event(event));
    let rewritten: BTreeSet<String> = rewrites.keys().cloned().collect();
    let event_ids: BTreeSet<String> = event_call_ids.iter().flatten().cloned().collect();
    let input = UndeliverableRewriteInput {
        call_ids,
        tool_call_item_count: tool_call_items,
        post_guardrail_tool_call_count: post.len(),
        unresolved_argument_event,
        rewritten_call_ids: &rewritten,
        event_call_ids: &event_ids,
    };
    undeliverable_reason(&input)
}

// ast-grep-ignore: no-long-names
pub fn deliver_ended_stream_tool_call_rewrites(
    earlier: &mut [Value],
    output: &mut [Value],
    pre: &[ToolCallShape],
    post: &[ToolCallShape],
    guardrail_name: &str,
) -> Result<(), StreamSyncError> {
    if post == pre {
        return Ok(());
    }
    let call_ids = tool_call_call_ids(output);
    let call_id_by_item_id = tool_call_ids_by_item_id(earlier);
    let event_call_ids: Vec<Option<String>> = earlier
        .iter()
        .map(|event| tool_call_event_call_id(event, &call_id_by_item_id))
        .collect();
    let rewrites = tool_call_rewrite_map(&call_ids, pre, post);
    let undeliverable = undeliverable_tool_call_rewrite(
        earlier,
        &call_ids,
        tool_call_item_count(output),
        post,
        &event_call_ids,
        &rewrites,
    );
    if let Some(reason) = undeliverable {
        return Err(StreamSyncError::Undeliverable {
            guardrail_name: guardrail_name.to_owned(),
            reason: reason.to_string(),
        });
    }
    write_item_rewrites(output, &call_ids, &rewrites);
    write_event_rewrites(earlier, &event_call_ids, &rewrites);
    Ok(())
}

fn write_item_rewrites(
    output: &mut [Value],
    call_ids: &[String],
    rewrites: &BTreeMap<String, ToolCallShape>,
) {
    let items = output
        .iter_mut()
        .filter(|item| is_tool_call_output_item(item));
    for (item, call_id) in items.zip(call_ids) {
        let Some(rewrite) = rewrites.get(call_id) else {
            continue;
        };
        write_tool_call_item(item, rewrite.name.as_deref(), Some(&rewrite.arguments));
    }
}

fn write_event_rewrites(
    earlier: &mut [Value],
    event_call_ids: &[Option<String>],
    rewrites: &BTreeMap<String, ToolCallShape>,
) {
    let mut delivered: BTreeSet<String> = BTreeSet::new();
    for (event, call_id) in earlier.iter_mut().zip(event_call_ids) {
        let Some(call_id) = call_id.as_deref() else {
            continue;
        };
        let Some(rewrite) = rewrites.get(call_id) else {
            continue;
        };
        write_rewrite_into_event(event, call_id, rewrite, &mut delivered);
    }
}

fn write_rewrite_into_event(
    event: &mut Value,
    call_id: &str,
    rewrite: &ToolCallShape,
    delivered: &mut BTreeSet<String>,
) {
    let Some(fields) = event.as_object_mut() else {
        return;
    };
    let Some(event_type) = fields
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return;
    };
    if TOOL_PAYLOAD_DELTA_EVENT_TYPES.contains(&event_type.as_str()) {
        let arguments = if delivered.insert(call_id.to_owned()) {
            rewrite.arguments.clone()
        } else {
            String::new()
        };
        fields.insert("delta".to_owned(), Value::String(arguments));
        return;
    }
    let done_field = TOOL_PAYLOAD_DONE_EVENT_FIELDS
        .iter()
        .find(|(event_name, _)| *event_name == event_type.as_str())
        .map(|(_, payload_field)| *payload_field);
    if let Some(payload_field) = done_field {
        let arguments = Value::String(rewrite.arguments.clone());
        fields.insert(payload_field.to_owned(), arguments);
        return;
    }
    if let Some(item) = fields.get_mut("item") {
        write_item_event(item, event_type.as_str(), rewrite);
    }
}

fn write_item_event(item: &mut Value, event_type: &str, rewrite: &ToolCallShape) {
    match event_type {
        "response.output_item.added" => write_tool_call_item(item, rewrite.name.as_deref(), None),
        "response.output_item.done" => {
            write_tool_call_item(item, rewrite.name.as_deref(), Some(&rewrite.arguments));
        }
        _ => {}
    }
}
