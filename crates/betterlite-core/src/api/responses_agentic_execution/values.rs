use betterlite_context::agentic_loop_plan::{AgenticLoopRequestPatch, FollowupMessage, JsonMap};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::value::to_raw_value;

use super::TOOLS_KEY;
use super::error::PatchError;
use crate::api::responses_hook_policy::safety::ApiSurface;

// RawValue and decisions::Json hold the same JSON, so the boundary only re-reads it
pub(crate) fn typed_map(map: &JsonMap) -> Result<IndexMap<String, Json>, serde_json::Error> {
    let mut typed = IndexMap::with_capacity(map.len());
    for (key, raw) in map {
        typed.insert(key.clone(), serde_json::from_str(raw.get())?);
    }
    Ok(typed)
}

pub(crate) fn raw_map(values: &IndexMap<String, Json>) -> Result<JsonMap, serde_json::Error> {
    let mut raw = JsonMap::with_capacity(values.len());
    for (key, value) in values {
        raw.insert(key.clone(), to_raw_value(value)?);
    }
    Ok(raw)
}

pub(crate) fn json_items(items: &[FollowupMessage]) -> Result<Json, serde_json::Error> {
    let mut list = Vec::with_capacity(items.len());
    for item in items {
        list.push(serde_json::from_str(item.0.get())?);
    }
    Ok(Json::List(list))
}

// the request's optional params, then the patch's over them, then the patch's tools
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

// absent and null both take the value the field already had, as docs/compatibility.md says
pub(crate) fn take_or(map: &mut IndexMap<String, Json>, key: &str, default: Json) -> Json {
    match map.shift_remove(key) {
        None | Some(Json::Null) => default,
        Some(value) => value,
    }
}

pub(crate) fn text_field(value: Json, field: &'static str) -> Result<String, PatchError> {
    match value {
        Json::Text(text) => Ok(text),
        _ => Err(PatchError::Field {
            field,
            expected: "a string",
        }),
    }
}

pub(crate) fn flag_field(value: Json, field: &'static str) -> Result<bool, PatchError> {
    match value {
        Json::Bool(flag) => Ok(flag),
        _ => Err(PatchError::Field {
            field,
            expected: "a boolean",
        }),
    }
}

pub(crate) fn count_field(value: Json, field: &'static str) -> Result<u64, PatchError> {
    match value {
        Json::Number(number) => number.as_u64().ok_or(PatchError::Field {
            field,
            expected: "a non-negative integer",
        }),
        _ => Err(PatchError::Field {
            field,
            expected: "a non-negative integer",
        }),
    }
}

pub(crate) fn message_list(
    value: Json,
    field: &'static str,
) -> Result<Vec<FollowupMessage>, PatchError> {
    match value {
        Json::List(items) => {
            let mut messages = Vec::with_capacity(items.len());
            for item in items {
                messages.push(FollowupMessage(to_raw_value(&item).map_err(|_| {
                    PatchError::Field {
                        field,
                        expected: "a list of messages",
                    }
                })?));
            }
            Ok(messages)
        }
        _ => Err(PatchError::Field {
            field,
            expected: "a list of messages",
        }),
    }
}

pub(crate) const fn api_surface_name(surface: ApiSurface) -> &'static str {
    match surface {
        ApiSurface::Responses => "responses",
        ApiSurface::AnthropicMessages => "anthropic_messages",
    }
}
