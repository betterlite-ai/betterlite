use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::response_id_codecs::response::decode_previous_response_id;

use super::warmup::is_warmup_response_id;
use super::{FrameSink, ManagedResponsesWebSocket};

#[must_use]
pub fn extract_response_id(completed: &IndexMap<String, Json>) -> Option<String> {
    let Some(Json::Object(response)) = completed.get("response") else {
        return None;
    };
    let Some(Json::Text(encoded_id)) = response.get("id") else {
        return None;
    };
    if encoded_id.is_empty() {
        return None;
    }
    Some(decode_previous_response_id(encoded_id))
}

// _apply_history: prepend the turn history, or hand the id back for B945's database reconstruction
pub fn apply_history(
    call_kwargs: &mut IndexMap<String, Json>,
    previous_response_id: Option<&str>,
    current_messages: &[IndexMap<String, Json>],
    prior_history: &[IndexMap<String, Json>],
) {
    let Some(raw_id) = previous_response_id.filter(|value| !value.is_empty()) else {
        return;
    };
    if is_warmup_response_id(Some(raw_id)) {
        tracing::debug!(
            "ManagedResponsesWS: ignoring synthetic warmup previous_response_id={raw_id}"
        );
        return;
    }
    if prior_history.is_empty() {
        tracing::debug!(
            "ManagedResponsesWS: no in-memory history for previous_response_id={raw_id}; falling back to DB-based session reconstruction"
        );
        call_kwargs.insert(
            "previous_response_id".to_owned(),
            Json::Text(raw_id.to_owned()),
        );
        return;
    }
    let count = prior_history.len();
    tracing::debug!(
        "ManagedResponsesWS: prepended {count} history messages for previous_response_id={raw_id}"
    );
    let combined = prior_history.iter().chain(current_messages);
    let input = combined
        .map(|message| Json::Object(message.clone()))
        .collect();
    call_kwargs.insert("input".to_owned(), Json::List(input));
}

#[must_use]
pub fn get_history_messages<Sink>(
    handler: &ManagedResponsesWebSocket<Sink>,
    previous_response_id: &str,
) -> Vec<IndexMap<String, Json>>
where
    Sink: FrameSink,
{
    let raw_id = decode_previous_response_id(previous_response_id);
    let stored = handler.session_history.get(&raw_id);
    stored.cloned().unwrap_or_default()
}
