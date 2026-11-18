use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::{StreamChunk, StreamItem};

const RAW_MODEL_NAME_KEY: &str = "_complexity_router_return_raw_model_name";
const FASTEST_RESPONSE_KEY: &str = "fastest_response";
const CALL_ID_KEY: &str = "litellm_call_id";

// the Azure Model Router test, handed in so the caller names it where the input is built
pub type RouterCheck = fn(&str) -> bool;

#[derive(Debug)]
pub struct RestampInput<'request> {
    pub requested_model: &'request str,
    pub request_data: &'request JsonObject,
    pub fallback_attempted: bool,
    pub fallback_model: Option<&'request str>,
    pub is_model_router: RouterCheck,
}

#[must_use]
pub fn should_return_raw_model_name(request_data: &JsonObject) -> bool {
    for key in ["metadata", "litellm_metadata"] {
        let Some(Json::Object(metadata)) = request_data.get(key) else {
            continue;
        };
        if matches!(metadata.get(RAW_MODEL_NAME_KEY), Some(Json::Bool(true))) {
            return true;
        }
    }
    false
}

// the chunk to stamp and the model it gets, or None for every skip rule
fn restamp_target<'item, 'request>(
    item: &'item mut StreamItem,
    input: &'request RestampInput<'_>,
) -> Option<(&'item mut StreamChunk, &'request str)> {
    if should_return_raw_model_name(input.request_data) {
        return None;
    }
    let target = if input.fallback_attempted {
        input.fallback_model
    } else {
        Some(input.requested_model)
    };
    let target = target.filter(|model| !model.is_empty())?;
    let StreamItem::Chunk(chunk) = item else {
        return None;
    };
    if !input.fallback_attempted && (input.is_model_router)(input.requested_model) {
        return None;
    }
    let fastest_response = input.request_data.get(FASTEST_RESPONSE_KEY);
    if !input.fallback_attempted && matches!(fastest_response, Some(Json::Bool(true))) {
        return None;
    }
    Some((chunk, target))
}

#[must_use]
pub fn restamp_chunk_model(
    item: &mut StreamItem,
    input: &RestampInput<'_>,
    mismatch_logged: bool,
) -> bool {
    let Some((chunk, target)) = restamp_target(item, input) else {
        return mismatch_logged;
    };
    if matches!(chunk.body.get("model"), Some(Json::Text(downstream)) if downstream == target) {
        return mismatch_logged;
    }
    if !mismatch_logged {
        log_mismatch(
            input.request_data,
            target,
            chunk.body.get("model"),
            input.fallback_attempted,
        );
    }
    chunk
        .body
        .insert("model".to_owned(), Json::Text(target.to_owned()));
    // a stamped chunk has always logged by now, whether this call logged it or an earlier one did
    true
}

fn log_mismatch(
    request_data: &JsonObject,
    target: &str,
    downstream: Option<&Json>,
    fallback_attempted: bool,
) {
    let call_id = match request_data.get(CALL_ID_KEY) {
        None | Some(Json::Null) => String::from("None"),
        Some(Json::Text(text)) => text.clone(),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
    };
    let downstream = match downstream {
        None | Some(Json::Null) => String::from("None"),
        Some(Json::Text(text)) => text.clone(),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
    };
    tracing::debug!(
        "litellm_call_id={call_id}: streaming chunk model mismatch - target={target} downstream={downstream} fallback_was_attempted={fallback_attempted}. Overriding chunk model to target."
    );
}
