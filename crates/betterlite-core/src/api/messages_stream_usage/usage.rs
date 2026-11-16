use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use crate::api::messages_chat_output::output::ContextManagement;
use crate::api::messages_chat_output::records::UsageRecord;
use crate::api::messages_chat_output::usage::project_usage;

use super::error::StreamStateError;
use super::events::json_from_serialize;
use super::state::StreamState;

// message_start announces all four cache counters as zero so clients see prompt caching supported
#[must_use]
pub fn initial_usage() -> Object {
    let mut usage = Object::new();
    usage.insert("input_tokens".to_owned(), Json::Number(Number::from(0_i64)));
    usage.insert(
        "output_tokens".to_owned(),
        Json::Number(Number::from(0_i64)),
    );
    usage.insert(
        "cache_creation_input_tokens".to_owned(),
        Json::Number(Number::from(0_i64)),
    );
    usage.insert(
        "cache_read_input_tokens".to_owned(),
        Json::Number(Number::from(0_i64)),
    );
    usage
}

// usage.get(key, 0) or 0 in the pinned augmentation: anything but an integer counter contributes nothing
fn counter(value: Option<&Json>) -> i64 {
    match value {
        Some(Json::Number(number)) => number.as_i64().unwrap_or(0),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => 0,
    }
}

impl<S> StreamState<S> {
    // Each applied edit list is surfaced once per message_delta, whether or not the caller already put one there
    fn attach_context_edits(&self, mut chunk: Object) -> Result<Object, StreamStateError> {
        if self.applied_edits.is_empty() || chunk.contains_key("context_management") {
            return Ok(chunk);
        }
        let management = ContextManagement {
            applied_edits: self.applied_edits.clone(),
        };
        chunk.insert(
            "context_management".to_owned(),
            json_from_serialize(&management)?,
        );
        Ok(chunk)
    }

    // Polyfill iterations belong to the final message_delta, and a usage without them has already been written
    pub fn augment_message_delta_usage(
        &self,
        mut chunk: Object,
    ) -> Result<Object, StreamStateError> {
        chunk = self.attach_context_edits(chunk)?;
        let Some(iterations) = &self.iterations_usage else {
            return Ok(chunk);
        };
        let found = chunk.get("usage").and_then(Json::as_object).cloned();
        let Some(mut usage) = found else {
            return Ok(chunk);
        };
        if usage.contains_key("iterations") {
            return Ok(chunk);
        }
        let input_tokens = counter(usage.get("input_tokens"));
        let output_tokens = counter(usage.get("output_tokens"));
        let mut appended: Vec<Json> = iterations
            .iter()
            .map(json_from_serialize)
            .collect::<Result<_, serde_json::Error>>()?;
        // A zero-token iteration says nothing true about the call the way the non-streaming path reports it
        if input_tokens > 0 || output_tokens > 0 {
            let mut iteration = Object::new();
            iteration.insert("type".to_owned(), Json::Text("message".to_owned()));
            iteration.insert(
                "input_tokens".to_owned(),
                Json::Number(Number::from(input_tokens)),
            );
            iteration.insert(
                "output_tokens".to_owned(),
                Json::Number(Number::from(output_tokens)),
            );
            appended.push(Json::Object(iteration));
        }
        usage.insert("iterations".to_owned(), Json::List(appended));
        chunk.insert("usage".to_owned(), Json::Object(usage));
        Ok(chunk)
    }

    // The held message_delta keeps its stop fields and takes this usage in place, never as a second usage
    pub fn merge_usage_into_held_stop(
        &self,
        usage: &UsageRecord,
    ) -> Result<Object, StreamStateError> {
        let Some(held) = &self.holding_stop_reason_chunk else {
            return Err(StreamStateError::MissingHeldStop);
        };
        let mut merged = held.clone();
        if !merged.contains_key("delta") {
            merged.insert("delta".to_owned(), Json::Object(Object::new()));
        }
        merged = self.attach_context_edits(merged)?;
        merged.insert(
            "usage".to_owned(),
            json_from_serialize(&project_usage(usage))?,
        );
        self.augment_message_delta_usage(merged)
    }

    // A metadata-only chunk only matters when the stop reason is held and the chunk carries usage
    pub fn handle_choiceless_chunk(
        &mut self,
        usage: Option<&UsageRecord>,
    ) -> Result<bool, StreamStateError> {
        if self.holding_stop_reason_chunk.is_none() {
            return Ok(false);
        }
        let Some(carried_usage) = usage else {
            return Ok(false);
        };
        let merged = self.merge_usage_into_held_stop(carried_usage)?;
        self.chunk_queue.push_back(merged);
        self.queued_usage_chunk = true;
        self.holding_stop_reason_chunk = None;
        Ok(true)
    }
}
