use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::{event_fields, object_field};
use crate::api::messages_agentic_stream::MessagesAgenticError;

// LiteLLM's _handle_message_start: id, model and role default to the current envelope values
pub(super) fn handle_message_start(
    data: &Json,
    response: &mut IndexMap<String, Json>,
) -> Result<(), MessagesAgenticError> {
    let fields = event_fields(data)?;
    let Some(message) = object_field(fields, "message")? else {
        return Ok(());
    };
    for key in ["id", "model", "role"] {
        if let Some(value) = message.get(key) {
            response.insert(key.to_owned(), value.clone());
        }
    }
    let Some(usage) = object_field(message, "usage")? else {
        return Ok(());
    };
    // LiteLLM's `if usage:` skips an empty usage object entirely, leaving earlier counts in place
    if usage.is_empty() {
        return Ok(());
    }
    let Some(Json::Object(response_usage)) = response.get_mut("usage") else {
        return Ok(());
    };
    let input_tokens = usage
        .get("input_tokens")
        .cloned()
        .unwrap_or(Json::Number(Number::from(0_u8)));
    response_usage.insert("input_tokens".to_owned(), input_tokens);
    for key in ["cache_creation_input_tokens", "cache_read_input_tokens"] {
        if let Some(value) = usage.get(key) {
            response_usage.insert(key.to_owned(), value.clone());
        }
    }
    Ok(())
}

// LiteLLM's _handle_message_delta: stop_reason, stop_sequence and the output usage arrive here
pub(super) fn handle_message_delta(
    data: &Json,
    response: &mut IndexMap<String, Json>,
) -> Result<(), MessagesAgenticError> {
    let fields = event_fields(data)?;
    let delta = object_field(fields, "delta")?.cloned().unwrap_or_default();
    for key in ["stop_reason", "stop_sequence"] {
        if let Some(value) = delta.get(key) {
            response.insert(key.to_owned(), value.clone());
        }
    }
    let usage = object_field(fields, "usage")?.cloned().unwrap_or_default();
    let Some(Json::Object(response_usage)) = response.get_mut("usage") else {
        return Ok(());
    };
    match usage.get("output_tokens") {
        None | Some(Json::Null) => {}
        Some(output_tokens) => {
            response_usage.insert("output_tokens".to_owned(), output_tokens.clone());
        }
    }
    for key in [
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
    ] {
        if let Some(value) = usage.get(key) {
            response_usage.insert(key.to_owned(), value.clone());
        }
    }
    Ok(())
}
