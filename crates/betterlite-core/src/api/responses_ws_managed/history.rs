use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::responses_ws_history::error::ResponsesWsHistoryError;
use crate::api::responses_ws_history::history::extract_response_id;
use crate::api::responses_ws_history::messages::extract_output_messages;
use crate::api::responses_ws_history::{FrameSink, ManagedResponsesWebSocket};

// _save_turn_history: one map write for a completed turn, keyed by the decoded response id
pub fn save_turn_history<Sink>(
    handler: &mut ManagedResponsesWebSocket<Sink>,
    completed: Option<&IndexMap<String, Json>>,
    prior_history: &[IndexMap<String, Json>],
    current_messages: &[IndexMap<String, Json>],
) -> Result<(), ResponsesWsHistoryError>
where
    Sink: FrameSink,
{
    let Some(completed) = completed else {
        return Ok(());
    };
    let Some(response_id) = extract_response_id(completed) else {
        return Ok(());
    };
    let output_messages = extract_output_messages(completed)?;
    let capacity = prior_history.len() + current_messages.len() + output_messages.len();
    let mut messages = Vec::with_capacity(capacity);
    messages.extend_from_slice(prior_history);
    messages.extend_from_slice(current_messages);
    messages.extend(output_messages);
    let stored = messages.len();
    handler
        .session_history
        .insert(response_id.clone(), messages);
    tracing::debug!("ManagedResponsesWS: stored {stored} messages for response_id={response_id}");
    Ok(())
}
