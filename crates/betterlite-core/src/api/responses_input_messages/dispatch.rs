use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Value;

use crate::api::responses_input_messages::Assembly;
use crate::api::responses_input_messages::error::InputMessagesError;
use crate::api::responses_input_messages::item::{
    is_function_call, is_tool_call_output, item_messages,
};
use crate::api::responses_reasoning_merge::merge_reasoning;
use crate::api::responses_reasoning_merge::record::ChatRecord;
use crate::api::responses_reasoning_merge::trailing::{TrailingInput, merge_trailing};
use crate::api::responses_tool_repair::definition::append_tool_call;
use crate::api::responses_tool_repair::value::Value as RepairValue;

pub(crate) fn append_items(
    messages: &mut Vec<Value>,
    items: &[Json],
    assembly: &Assembly<'_>,
) -> Result<(), InputMessagesError> {
    let mut call_ids: BTreeSet<String> = BTreeSet::new();
    for (position, item) in items.iter().enumerate() {
        let Json::Object(fields) = item else {
            return Err(InputMessagesError::ItemNotObject { position });
        };
        let produced = item_messages(fields, assembly)?;
        let kind = match fields.get("type") {
            Some(Json::Text(text)) => Some(text.as_str()),
            None
            | Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            ) => None,
        };
        if is_function_call(kind) {
            add_call_id(&mut call_ids, fields);
            if last_is_assistant(messages) {
                append_to_last(messages, &produced)?;
                continue;
            }
        }
        if is_tool_call_output(kind) {
            append_tool_outputs(messages, produced, &mut call_ids);
            continue;
        }
        append_or_merge(messages, produced, kind == Some("web_search_call"))?;
    }
    if assembly.replay_reasoning {
        merge_reasoning_messages(messages)?;
    }
    Ok(())
}

// B912 rejects a non-text call id while building the call, so only a text id reaches this set
fn add_call_id(call_ids: &mut BTreeSet<String>, fields: &IndexMap<String, Json>) {
    let direct = match fields.get("call_id") {
        Some(Json::Text(id)) if !id.is_empty() => Some(id.as_str()),
        None
        | Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_)
            | Json::Text(_),
        ) => None,
    };
    let fallback = match fields.get("id") {
        Some(Json::Text(id)) if !id.is_empty() => Some(id.as_str()),
        None
        | Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_)
            | Json::Text(_),
        ) => None,
    };
    if let Some(id) = direct.or(fallback) {
        call_ids.insert(id.to_owned());
    }
}

fn last_is_assistant(messages: &[Value]) -> bool {
    matches!(
        messages.last(),
        Some(Value::Object(fields)) if fields.get("role").and_then(Value::as_str) == Some("assistant")
    )
}

fn append_to_last(messages: &mut [Value], produced: &[Value]) -> Result<(), InputMessagesError> {
    let Some(last) = messages.last_mut() else {
        return Ok(());
    };
    for message in produced {
        let Value::Object(fields) = message else {
            continue;
        };
        if fields.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(Value::Array(calls)) = fields.get("tool_calls") else {
            continue;
        };
        for call in calls {
            append_call(last, call)?;
        }
    }
    Ok(())
}

fn append_call(assistant: &mut Value, call: &Value) -> Result<(), InputMessagesError> {
    let mut target: RepairValue = serde_json::from_value(assistant.clone())?;
    append_tool_call(&mut target, serde_json::from_value(call.clone())?);
    *assistant = serde_json::to_value(&target)?;
    Ok(())
}

fn append_tool_outputs(
    messages: &mut Vec<Value>,
    produced: Vec<Value>,
    call_ids: &mut BTreeSet<String>,
) {
    for message in produced {
        if let Some(id) = first_call_id(&message) {
            if call_ids.contains(id) {
                continue;
            }
            call_ids.insert(id.to_owned());
        }
        messages.push(message);
    }
}

// Only the first call of the cached assistant wrapper names the call LiteLLM compares
fn first_call_id(message: &Value) -> Option<&str> {
    let Value::Object(fields) = message else {
        return None;
    };
    if fields.get("role").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let Some(Value::Array(calls)) = fields.get("tool_calls") else {
        return None;
    };
    let Value::Object(first) = calls.first()? else {
        return None;
    };
    match first.get("id") {
        Some(Value::String(id)) if !id.is_empty() => Some(id.as_str()),
        None
        | Some(
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::Array(_)
            | Value::Object(_)
            | Value::String(_),
        ) => None,
    }
}

fn append_or_merge(
    messages: &mut Vec<Value>,
    produced: Vec<Value>,
    hosted_search: bool,
) -> Result<(), InputMessagesError> {
    if let Some(merged) = merged_trailing(messages.last(), &produced, hosted_search)? {
        if let Some(last) = messages.last_mut() {
            *last = merged;
        }
        return Ok(());
    }
    messages.extend(produced);
    Ok(())
}

fn merged_trailing(
    last: Option<&Value>,
    produced: &[Value],
    hosted_search: bool,
) -> Result<Option<Value>, InputMessagesError> {
    let Some(previous_message) = last else {
        return Ok(None);
    };
    let previous = [TrailingInput::from_value(previous_message.clone())?];
    let added = produced
        .iter()
        .cloned()
        .map(TrailingInput::from_value)
        .collect::<Result<Vec<_>, _>>()?;
    let merged = merge_trailing(&previous, &added, hosted_search)?;
    merged
        .map(serde_json::to_value)
        .transpose()
        .map_err(InputMessagesError::from)
}

fn merge_reasoning_messages(messages: &mut Vec<Value>) -> Result<(), InputMessagesError> {
    let mut records = messages
        .iter()
        .cloned()
        .map(serde_json::from_value::<ChatRecord>)
        .collect::<Result<Vec<_>, _>>()?;
    let merged = merge_reasoning(&mut records);
    *messages = merged
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<_, _>>()?;
    Ok(())
}
