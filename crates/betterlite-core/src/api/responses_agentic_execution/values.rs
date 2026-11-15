use super::TOOLS_KEY;
use super::error::PatchError;
use crate::api::responses_hook_policy::safety::ApiSurface;
use betterlite_context::agentic_loop_plan::{AgenticLoopRequestPatch, FollowupMessage, JsonMap};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::value::to_raw_value;
pub(crate) fn typed_map(map: &JsonMap) -> Result<IndexMap<String, Json>, serde_json::Error> {
    let mut typed = IndexMap::with_capacity(map.len());
    for (key, raw) in map {
        typed.insert(key.clone(), serde_json::from_str(raw.get())?);
    }
    Ok(typed)
}
pub(crate) fn json_items(items: &[FollowupMessage]) -> Result<Json, serde_json::Error> {
    let mut list = Vec::with_capacity(items.len());
    for item in items {
        list.push(serde_json::from_str(item.0.get())?);
    }
    Ok(Json::List(list))
}
pub(crate) fn merged_params(
    optional_params: &JsonMap,
    patch: &AgenticLoopRequestPatch,
) -> Result<IndexMap<String, Json>, PatchError> {
    let mut params = typed_map(optional_params)?;
    for (key, value) in &patch.optional_params {
        params.insert(key.clone(), serde_json::from_str(value.get())?);
    }
    if let Some(tools) = &patch.tools {
        params.insert(TOOLS_KEY.to_owned(), json_items(tools)?);
    }
    Ok(params)
}
pub(crate) fn take_or(map: &mut IndexMap<String, Json>, key: &str, default: Json) -> Json {
    match map.shift_remove(key) {
        None | Some(Json::Null) => default,
        Some(value) => value,
    }
}
pub(crate) fn text_field(value: Json, field: &'static str) -> Result<String, PatchError> {
    if let Json::Text(text) = value {
        return Ok(text);
    }
    Err(PatchError::Field {
        field,
        expected: "a string",
    })
}
pub(crate) fn count_field(value: Json, field: &'static str) -> Result<u64, PatchError> {
    let Json::Number(number) = value else {
        return Err(PatchError::Field {
            field,
            expected: "a non-negative integer",
        });
    };
    number.as_u64().ok_or_else(|| PatchError::Field {
        field,
        expected: "a non-negative integer",
    })
}
pub(crate) fn message_list(
    value: Json,
    field: &'static str,
) -> Result<Vec<FollowupMessage>, PatchError> {
    let Json::List(items) = value else {
        return Err(PatchError::Field {
            field,
            expected: "a list of messages",
        });
    };
    let mut messages = Vec::with_capacity(items.len());
    for item in items {
        messages.push(FollowupMessage(to_raw_value(&item)?));
    }
    Ok(messages)
}
pub(crate) const fn api_surface_name(surface: ApiSurface) -> &'static str {
    match surface {
        ApiSurface::Responses => "responses",
        ApiSurface::AnthropicMessages => "anthropic_messages",
    }
}
