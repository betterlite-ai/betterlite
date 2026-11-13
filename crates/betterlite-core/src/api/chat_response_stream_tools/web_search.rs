use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::Extra;
use betterlite_openai::api::responses_objects::sdk::search::WebSearch;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::error::StreamToolsError;
use super::events::{WebSearchStage, output_item_event, web_search_event};
use super::fields::object;
use super::state::{ChatStreamState, QueuedToolEvent, assign_tool_output_index, next_sequence};

const WEB_SEARCH_CALL_TYPE: &str = "web_search_call";
const WEB_SEARCH_PREFIX: &str = "ws_";
const IN_PROGRESS: &str = "in_progress";

pub fn reserve_web_search_indexes<Context>(
    state: &mut ChatStreamState<Context>,
    provider_fields: &Json,
) -> Result<(), StreamToolsError> {
    let Some(calls) = object(provider_fields).and_then(|fields| fields.get("web_search_calls"))
    else {
        return Ok(());
    };
    let items: Vec<&Json> = match calls {
        Json::Object(by_key) => by_key.values().collect(),
        Json::List(listed) => listed.iter().collect(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => Vec::new(),
    };
    for item in items {
        reserve_web_search_item(state, item)?;
    }
    Ok(())
}

fn reserve_web_search_item<Context>(
    state: &mut ChatStreamState<Context>,
    item: &Json,
) -> Result<(), StreamToolsError> {
    let Some(fields) = object(item) else {
        return Ok(());
    };
    let Some(Json::Text(id)) = fields.get("id") else {
        return Ok(());
    };
    let call_id = id.strip_prefix(WEB_SEARCH_PREFIX).unwrap_or(id).to_owned();
    if call_id.is_empty() {
        return Ok(());
    }
    let output_index = assign_tool_output_index(state, &call_id)?;
    let in_progress =
        matches!(fields.get("status"), Some(Json::Text(status)) if status == IN_PROGRESS);
    state
        .progress
        .web_search_calls
        .insert(call_id, item.clone());
    if in_progress {
        state
            .progress
            .pending_tool_events
            .retain(|queued| queued.output_index != output_index);
    }
    Ok(())
}

pub fn queue_web_search_events<Context>(
    state: &mut ChatStreamState<Context>,
    call_id: &str,
    item: &Json,
) -> Result<(), StreamToolsError> {
    let call = web_search_json(item)?;
    let output_index = assign_tool_output_index(state, call_id)?;
    let added_sequence = next_sequence(state)?;
    state.progress.pending_tool_events.push(QueuedToolEvent {
        output_index,
        event: output_item_event(
            output_index,
            web_search_added_fields(&call),
            added_sequence,
            false,
        ),
    });
    for stage in [
        WebSearchStage::InProgress,
        WebSearchStage::Searching,
        WebSearchStage::Completed,
    ] {
        let stage_sequence = next_sequence(state)?;
        state.progress.pending_tool_events.push(QueuedToolEvent {
            output_index,
            event: web_search_event(stage, output_index, &call.id, stage_sequence),
        });
    }
    let done_sequence = next_sequence(state)?;
    state.progress.pending_tool_events.push(QueuedToolEvent {
        output_index,
        event: output_item_event(output_index, web_search_json(&call)?, done_sequence, true),
    });
    Ok(())
}

fn web_search_json<T, U>(value: &T) -> Result<U, StreamToolsError>
where
    T: Serialize,
    U: DeserializeOwned,
{
    let serialized =
        serde_json::to_value(value).map_err(|source| StreamToolsError::WebSearch { source })?;
    serde_json::from_value(serialized).map_err(|source| StreamToolsError::WebSearch { source })
}

fn web_search_added_fields(call: &WebSearch) -> Extra {
    let mut fields = Extra::new();
    fields.insert(String::from("id"), Json::Text(call.id.clone()));
    fields.insert(
        String::from("type"),
        Json::Text(WEB_SEARCH_CALL_TYPE.to_owned()),
    );
    fields.insert(String::from("status"), Json::Text(IN_PROGRESS.to_owned()));
    fields.insert(String::from("action"), Json::Null);
    fields
}
