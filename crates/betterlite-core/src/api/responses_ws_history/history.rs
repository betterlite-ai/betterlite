use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::response_id_codecs::response::decode_previous_response_id;

use super::warmup::is_warmup_response_id;
use super::{FrameSink, ManagedResponsesWebSocket};

// _extract_response_id: the decoded provider id inside a response.completed event
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
    let Some(previous_response_id) = previous_response_id.filter(|value| !value.is_empty()) else {
        return;
    };
    if is_warmup_response_id(Some(previous_response_id)) {
        tracing::debug!(
            "ManagedResponsesWS: ignoring synthetic warmup previous_response_id={previous_response_id}"
        );
        return;
    }
    if prior_history.is_empty() {
        tracing::debug!(
            "ManagedResponsesWS: no in-memory history for previous_response_id={previous_response_id}; falling back to DB-based session reconstruction"
        );
        call_kwargs.insert(
            "previous_response_id".to_owned(),
            Json::Text(previous_response_id.to_owned()),
        );
        return;
    }
    tracing::debug!(
        "ManagedResponsesWS: prepended {count} history messages for previous_response_id={previous_response_id}",
        count = prior_history.len()
    );
    let mut input = Vec::with_capacity(prior_history.len().saturating_add(current_messages.len()));
    input.extend(
        prior_history
            .iter()
            .chain(current_messages)
            .map(|message| Json::Object(message.clone())),
    );
    call_kwargs.insert("input".to_owned(), Json::List(input));
}

impl<Sink: FrameSink> ManagedResponsesWebSocket<Sink> {
    // _store_history: replaces the whole turn list for the raw provider id
    pub fn store_history(&mut self, response_id: &str, messages: Vec<IndexMap<String, Json>>) {
        self.session_history
            .insert(response_id.to_owned(), messages);
    }

    // _get_history_messages: a copy, so a caller appending to the result never edits the stored turn
    #[must_use]
    pub fn get_history_messages(&self, previous_response_id: &str) -> Vec<IndexMap<String, Json>> {
        let raw_id = decode_previous_response_id(previous_response_id);
        self.session_history
            .get(&raw_id)
            .cloned()
            .unwrap_or_else(Vec::new)
    }
}
