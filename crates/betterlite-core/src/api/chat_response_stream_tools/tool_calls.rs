use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::Extra;

use super::error::StreamToolsError;
use super::events::{function_call_arguments_event, output_item_event};
use super::fields::{list, object, string_value, text_or_empty};
use super::state::{ChatStreamState, QueuedToolEvent, assign_tool_output_index, next_sequence};
use crate::api::responses_codex_tools::custom_tools::{
    ToolCall, ToolCallItem, ToolCallPayload, serialize_tool_call_arguments, tool_call_item,
};
use crate::api::responses_tool_namespaces::namespace_names::restore_name;
use crate::api::responses_tool_namespaces::values::has_value;

const ARGUMENT_CHUNK_CHARS: usize = 10;

#[must_use]
pub fn normalize_tool_call_index(tool_call: &Json) -> Option<i64> {
    let value = object(tool_call).and_then(|fields| fields.get("index"))?;
    match value {
        Json::Bool(true) => Some(1),
        Json::Bool(false) => Some(0),
        Json::Number(number) => number.as_i64().or_else(|| {
            number
                .as_f64()
                .filter(|float| float.is_finite())
                .and_then(num_traits::cast)
        }),
        Json::Text(text) => text.trim().parse::<i64>().ok(),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}

pub(super) fn argument_chunks(arguments: &str) -> Vec<String> {
    arguments
        .chars()
        .collect::<Vec<_>>()
        .chunks(ARGUMENT_CHUNK_CHARS)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

pub fn tool_call_id<Context>(state: &mut ChatStreamState<Context>, tool_call: &Json) -> String {
    let index = normalize_tool_call_index(tool_call);
    let Some(id_field) = object(tool_call)
        .and_then(|fields| fields.get("id"))
        .filter(|value| has_value(value))
    else {
        return known_call_id(state, index);
    };
    let call_id = string_value(id_field);
    if let Some(position) = index {
        let conflicting = state
            .progress
            .tool_call_id_by_index
            .get(&position)
            .is_some_and(|existing| existing != &call_id);
        if conflicting {
            state.progress.ambiguous_tool_call_indexes.insert(position);
        }
        state
            .progress
            .tool_call_id_by_index
            .insert(position, call_id.clone());
    }
    call_id
}

fn known_call_id<Context>(state: &ChatStreamState<Context>, index: Option<i64>) -> String {
    let Some(position) = index else {
        return String::new();
    };
    if state
        .progress
        .ambiguous_tool_call_indexes
        .contains(&position)
    {
        return String::new();
    }
    state
        .progress
        .tool_call_id_by_index
        .get(&position)
        .cloned()
        .unwrap_or_default()
}

pub(super) fn tool_call_name_and_arguments(call: &Json) -> (String, String) {
    let function = object(call)
        .and_then(|fields| fields.get("function"))
        .and_then(object);
    let name = text_or_empty(function.and_then(|fields| fields.get("name")));
    let arguments = serialize_tool_call_arguments(
        function
            .and_then(|fields| fields.get("arguments"))
            .unwrap_or(&Json::Null),
        "",
    );
    (name, arguments)
}

pub fn queue_tool_delta_events<Context>(
    state: &mut ChatStreamState<Context>,
    tool_calls: &Json,
) -> Result<(), StreamToolsError> {
    let Some(calls) = list(tool_calls) else {
        return Ok(());
    };
    for call in calls {
        queue_tool_delta(state, call)?;
    }
    Ok(())
}

fn queue_tool_delta<Context>(
    state: &mut ChatStreamState<Context>,
    call: &Json,
) -> Result<(), StreamToolsError> {
    let call_id = tool_call_id(state, call);
    if call_id.is_empty() || state.progress.web_search_calls.contains_key(&call_id) {
        return Ok(());
    }
    let (fn_name, arguments) = tool_call_name_and_arguments(call);
    let output_index = assign_tool_output_index(state, &call_id)?;
    if !state.progress.tool_args_by_call_id.contains_key(&call_id) {
        queue_tool_item_added(state, &call_id, &fn_name, output_index)?;
    }
    if arguments.is_empty() {
        return Ok(());
    }
    if let Some(accumulated) = state.progress.tool_args_by_call_id.get_mut(&call_id) {
        accumulated.push_str(&arguments);
    }
    for chunk in argument_chunks(&arguments) {
        let sequence = next_sequence(state)?;
        let item_id = tool_item_id(state, &call_id);
        state.progress.pending_tool_events.push(QueuedToolEvent {
            output_index,
            event: function_call_arguments_event(&item_id, output_index, &chunk, sequence, false),
        });
    }
    Ok(())
}

pub(super) fn queue_tool_item_added<Context>(
    state: &mut ChatStreamState<Context>,
    call_id: &str,
    fn_name: &str,
    output_index: i64,
) -> Result<(), StreamToolsError> {
    state
        .progress
        .tool_args_by_call_id
        .insert(call_id.to_owned(), String::new());
    let sequence = next_sequence(state)?;
    let (item_id, item) = tool_call_fields(state, call_id, fn_name, "", "in_progress")?;
    state
        .progress
        .tool_item_id_by_call_id
        .insert(call_id.to_owned(), item_id);
    state.progress.pending_tool_events.push(QueuedToolEvent {
        output_index,
        event: output_item_event(output_index, item, sequence, false),
    });
    Ok(())
}

pub fn tool_item_id<Context>(state: &ChatStreamState<Context>, call_id: &str) -> String {
    state
        .progress
        .tool_item_id_by_call_id
        .get(call_id)
        .cloned()
        .unwrap_or_else(|| call_id.to_owned())
}

pub(super) fn tool_call_fields<Context>(
    state: &ChatStreamState<Context>,
    call_id: &str,
    fn_name: &str,
    arguments: &str,
    status: &str,
) -> Result<(String, Extra), StreamToolsError> {
    let item = tool_call_item(
        ToolCall {
            call_id,
            name: fn_name,
            arguments_or_input: arguments,
            status,
        },
        &state.custom_tool_names,
        state.limits,
    )?;
    if state.custom_tool_names.contains(fn_name) {
        return Ok(item_fields(item, fn_name, None));
    }
    let (restored, namespace) = restore_name(fn_name, &state.namespace_tool_names);
    Ok(item_fields(item, &restored, namespace.as_deref()))
}

fn item_fields(item: ToolCallItem, name: &str, tool_namespace: Option<&str>) -> (String, Extra) {
    let id = item.id.clone();
    let mut fields = Extra::new();
    fields.insert(String::from("type"), Json::Text(item.kind.to_owned()));
    fields.insert(String::from("id"), Json::Text(item.id));
    fields.insert(String::from("call_id"), Json::Text(item.call_id));
    fields.insert(String::from("name"), Json::Text(name.to_owned()));
    fields.insert(String::from("status"), Json::Text(item.status));
    match item.payload {
        ToolCallPayload::Function { arguments } => {
            fields.insert(String::from("arguments"), Json::Text(arguments));
        }
        ToolCallPayload::Custom { input } => {
            fields.insert(String::from("input"), Json::Text(input));
        }
    }
    if let Some(present_namespace) = tool_namespace {
        fields.insert(
            String::from("namespace"),
            Json::Text(present_namespace.to_owned()),
        );
    }
    (id, fields)
}
