use betterlite_models::decisions::Json;

use super::error::StreamToolsError;
use super::events::{function_call_arguments_event, output_item_event};
use super::fields::{list, object, string_value};
use super::state::{ChatStreamState, QueuedToolEvent, assign_tool_output_index, next_sequence};
use super::tool_calls::{
    argument_chunks, queue_tool_item_added, tool_call_fields, tool_call_name_and_arguments,
    tool_item_id,
};
use crate::api::responses_tool_namespaces::values::has_value;

pub fn queue_final_tool_completion<Context>(
    state: &mut ChatStreamState<Context>,
    completion: &Json,
) -> Result<(), StreamToolsError> {
    if state.progress.final_tool_events_queued {
        return Ok(());
    }
    state.progress.final_tool_events_queued = true;
    let Some(Json::List(tool_calls)) = final_tool_calls(completion) else {
        return Ok(());
    };
    for call in tool_calls {
        queue_final_call(state, call)?;
    }
    Ok(())
}

fn queue_final_call<Context>(
    state: &mut ChatStreamState<Context>,
    call: &Json,
) -> Result<(), StreamToolsError> {
    let Some(id_field) = object(call)
        .and_then(|fields| fields.get("id"))
        .filter(|value| has_value(value))
    else {
        return Ok(());
    };
    let call_id = string_value(id_field);
    let output_index = assign_tool_output_index(state, &call_id)?;
    let (fn_name, arguments) = tool_call_name_and_arguments(call);
    if state.progress.web_search_calls.contains_key(&call_id) {
        return queue_final_web_search(state, &call_id);
    }
    if !state
        .progress
        .tool_item_id_by_call_id
        .contains_key(&call_id)
    {
        queue_tool_item_added(state, &call_id, &fn_name, output_index)?;
    }
    let final_args = if arguments.is_empty() {
        state
            .progress
            .tool_args_by_call_id
            .get(&call_id)
            .cloned()
            .unwrap_or_default()
    } else {
        arguments
    };
    queue_final_arguments(state, &call_id, output_index, &final_args)?;
    queue_final_item_done(state, &call_id, &fn_name, output_index, &final_args)
}

pub(super) fn queue_final_web_search<Context>(
    state: &mut ChatStreamState<Context>,
    call_id: &str,
) -> Result<(), StreamToolsError> {
    if state.progress.queued_web_search_call_ids.contains(call_id) {
        return Ok(());
    }
    let Some(item) = state.progress.web_search_calls.get(call_id).cloned() else {
        return Ok(());
    };
    super::web_search::queue_web_search_events(state, call_id, &item)?;
    state
        .progress
        .queued_web_search_call_ids
        .insert(call_id.to_owned());
    Ok(())
}

fn queue_final_arguments<Context>(
    state: &mut ChatStreamState<Context>,
    call_id: &str,
    output_index: i64,
    final_args: &str,
) -> Result<(), StreamToolsError> {
    let streamed = state
        .progress
        .tool_args_by_call_id
        .get(call_id)
        .map_or(0, |text| text.chars().count());
    let remaining: String = final_args.chars().skip(streamed).collect();
    for chunk in argument_chunks(&remaining) {
        let sequence = next_sequence(state)?;
        let item_id = tool_item_id(state, call_id);
        state.progress.pending_tool_events.push(QueuedToolEvent {
            output_index,
            event: function_call_arguments_event(&item_id, output_index, &chunk, sequence, false),
        });
    }
    let sequence = next_sequence(state)?;
    let item_id = tool_item_id(state, call_id);
    state.progress.pending_tool_events.push(QueuedToolEvent {
        output_index,
        event: function_call_arguments_event(&item_id, output_index, final_args, sequence, true),
    });
    Ok(())
}

fn queue_final_item_done<Context>(
    state: &mut ChatStreamState<Context>,
    call_id: &str,
    fn_name: &str,
    output_index: i64,
    final_args: &str,
) -> Result<(), StreamToolsError> {
    let sequence = next_sequence(state)?;
    let (built_id, mut item) = tool_call_fields(state, call_id, fn_name, final_args, "completed")?;
    let item_id = state
        .progress
        .tool_item_id_by_call_id
        .entry(call_id.to_owned())
        .or_insert(built_id)
        .clone();
    item.insert(String::from("id"), Json::Text(item_id));
    state.progress.pending_tool_events.push(QueuedToolEvent {
        output_index,
        event: output_item_event(output_index, item, sequence, true),
    });
    Ok(())
}

fn final_tool_calls(completion: &Json) -> Option<&Json> {
    let choices = list(object(completion)?.get("choices")?)?;
    let message = object(choices.first()?)?.get("message")?;
    object(message)?.get("tool_calls")
}
