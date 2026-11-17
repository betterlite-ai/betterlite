#![expect(
    clippy::disallowed_types,
    clippy::too_many_arguments,
    reason = "Responses stream events are provider JSON that betterlite does not model, and the undeliverable check keeps LiteLLM's argument list"
)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        reason = "serde_json test fixtures built with json! are concise and type-directed"
    )
)]

mod delivery;
pub mod error;
mod positions;
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_message_scope::fingerprint::stream_item_field;
use serde_json::Value;

use crate::guardrails::responses_translation::input_provenance::is_tool_call_output_item;
use crate::guardrails::responses_translation::stream_scan::{
    OUTPUT_ITEM_EVENT_TYPES, TOOL_PAYLOAD_EVENT_TYPES, event_item, integer_field,
    is_tool_call_stream_item, item_text,
};
use crate::guardrails::tool_call_identity::ToolCallShape;
use delivery::{
    tool_call_call_ids,
    tool_call_rewrite_map,
    // ast-grep-ignore: no-long-names
    undeliverable_tool_call_rewrite,
    write_event_rewrites,
    write_item_rewrites,
};
use error::StreamSyncError;
use positions::{placeable_positions, scanned_text_positions};

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
    // ast-grep-ignore: no-long-names
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
        let Some(inner) = stream_item_field(&item, "item")
            .cloned()
            .map(StreamItem::Json)
        else {
            continue;
        };
        if !is_tool_call_stream_item(&inner) {
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
    let inner = stream_item_field(&item, "item")
        .cloned()
        .map(StreamItem::Json)?;
    if !is_tool_call_stream_item(&inner) {
        return None;
    }
    item_text(&inner, "call_id").map(str::to_owned)
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
    let tool_call_items = output
        .iter()
        .filter(|item| is_tool_call_output_item(item))
        .count();
    // ast-grep-ignore: no-long-names
    let undeliverable = undeliverable_tool_call_rewrite(
        earlier,
        &call_ids,
        tool_call_items,
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
