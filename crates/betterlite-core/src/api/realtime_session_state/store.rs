use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::DEFAULT_LOGGED_EVENT_TYPES;
use super::RealtimeStreaming;
use super::error::SessionStateError;
use super::records::{check_session_event, decoded_record, event_type, tool_call_record};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMessage<'message> {
    Text(&'message str),
    Binary(&'message [u8]),
    Record(&'message IndexMap<String, Json>),
}

// LiteLLM collects response.done tool calls before it applies the event filter, so an unlogged response still bills its functions
pub(crate) fn store_message(
    state: &mut RealtimeStreaming,
    message: SessionMessage<'_>,
) -> Result<(), SessionStateError> {
    let record = match message {
        SessionMessage::Record(record) => record.clone(),
        SessionMessage::Text(text) => decoded_record(text.as_bytes())?,
        SessionMessage::Binary(bytes) => decoded_record(bytes)?,
    };
    collect_response_tool_calls(state, &record);
    if should_store_message(state, &record) {
        if let Err(error) = check_session_event(&record) {
            tracing::debug!("Error parsing message for logging: {error}");
        }
        state.messages.push(record);
    }
    Ok(())
}

#[must_use]
pub(crate) fn should_store_message(
    state: &RealtimeStreaming,
    record: &IndexMap<String, Json>,
) -> bool {
    match &state.logged_real_time_event_types {
        LoggedEventTypes::All => true,
        LoggedEventTypes::Default => {
            event_type(record).is_some_and(|event| DEFAULT_LOGGED_EVENT_TYPES.contains(&event))
        }
        LoggedEventTypes::Only(types) => {
            event_type(record).is_some_and(|event| types.iter().any(|logged| logged == event))
        }
    }
}

pub(crate) fn collect_response_tool_calls(
    state: &mut RealtimeStreaming,
    event: &IndexMap<String, Json>,
) {
    if event_type(event) != Some("response.done") {
        return;
    }
    let Some(Json::Object(response)) = event.get("response") else {
        return;
    };
    let Some(Json::List(output)) = response.get("output") else {
        return;
    };
    for item in output {
        let Json::Object(entry) = item else {
            // LiteLLM reads item.get on every entry, and a non-object entry ends its loop
            break;
        };
        if matches!(entry.get("type"), Some(Json::Text(kind)) if kind == "function_call") {
            state.tool_calls.push(tool_call_record(entry));
        }
    }
}
