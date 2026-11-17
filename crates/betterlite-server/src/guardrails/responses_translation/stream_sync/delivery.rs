use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::guardrails::responses_translation::input_provenance::is_tool_call_output_item;
use crate::guardrails::responses_translation::output_rewrite::write_tool_call_item;
use crate::guardrails::responses_translation::stream_scan::{
    TOOL_PAYLOAD_DELTA_EVENT_TYPES, TOOL_PAYLOAD_DONE_EVENT_FIELDS, TOOL_PAYLOAD_EVENT_TYPES,
    event_item, item_text,
};
use crate::guardrails::tool_call_identity::error::UndeliverableRewriteReason;
use crate::guardrails::tool_call_identity::{
    ToolCallShape, UndeliverableRewriteInput, tool_call_rewrite, undeliverable_reason,
};

fn is_payload_event(event: &Value) -> bool {
    let Some(item) = event_item(event) else {
        return false;
    };
    item_text(&item, "type")
        .is_some_and(|event_type| TOOL_PAYLOAD_EVENT_TYPES.contains(&event_type))
}

pub(super) fn tool_call_item_count(output: &[Value]) -> usize {
    output
        .iter()
        .filter(|item| is_tool_call_output_item(item))
        .count()
}

pub(super) fn tool_call_call_ids(output: &[Value]) -> Vec<String> {
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

pub(super) fn tool_call_rewrite_map(
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

// ast-grep-ignore: no-long-names
pub(super) fn undeliverable_tool_call_rewrite(
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

pub(super) fn write_item_rewrites(
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

pub(super) fn write_event_rewrites(
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
