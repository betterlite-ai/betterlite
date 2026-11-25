use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::responses_ws_history::error::ResponsesWsHistoryError;
use crate::api::responses_ws_history::history::extract_response_id;
use crate::api::responses_ws_history::messages::extract_output_messages;
use crate::api::responses_ws_history::{FrameSink, ManagedResponsesWebSocket};

pub fn save_turn_history<Sink>(
    handler: &mut ManagedResponsesWebSocket<Sink>,
    completed: Option<&IndexMap<String, Json>>,
    prior_history: &[IndexMap<String, Json>],
    current_messages: &[IndexMap<String, Json>],
) -> Result<(), ResponsesWsHistoryError>
where
    Sink: FrameSink,
{
    let Some(turn) = completed else {
        return Ok(());
    };
    let Some(response_id) = extract_response_id(turn) else {
        return Ok(());
    };
    let output_messages = extract_output_messages(turn)?;
    let capacity = prior_history
        .len()
        .saturating_add(current_messages.len())
        .saturating_add(output_messages.len());
    let mut messages = Vec::with_capacity(capacity);
    messages.extend_from_slice(prior_history);
    messages.extend_from_slice(current_messages);
    messages.extend(output_messages);
    handler.store_history(response_id, messages);
    Ok(())
}
