use std::collections::{BTreeMap, BTreeSet};

use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use super::error::AgenticLoopSafetyError;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefusedToolIdentifiers {
    pub ids: BTreeSet<String>,
    pub names: BTreeSet<String>,
}

#[must_use]
pub fn sort_json_keys(value: &Json) -> Json {
    match value {
        Json::Object(object) => {
            let sorted: BTreeMap<&str, Json> = object
                .iter()
                .map(|(key, item)| (key.as_str(), sort_json_keys(item)))
                .collect();
            let ordered: IndexMap<String, Json> = sorted
                .into_iter()
                .map(|(key, item)| (key.to_owned(), item))
                .collect();
            Json::Object(ordered)
        }
        Json::List(items) => Json::List(items.iter().map(sort_json_keys).collect()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => value.clone(),
    }
}

pub fn check_agentic_loop_safety(
    tool_calls: &Json,
    fingerprints: &[Json],
    depth: i64,
    max_loops: u32,
    model: &str,
) -> Result<String, AgenticLoopSafetyError> {
    let canonical = sort_json_keys(tool_calls);
    let fingerprint = python_json::dumps(&canonical);
    let repeated = fingerprints
        .iter()
        .any(|seen| matches!(seen, Json::Text(text) if text == &fingerprint));
    if repeated {
        return Err(AgenticLoopSafetyError::RepeatedFingerprint);
    }
    if depth >= i64::from(max_loops) {
        return Err(AgenticLoopSafetyError::MaxLoopsExceeded {
            max_loops,
            model: model.to_owned(),
        });
    }
    Ok(fingerprint)
}

#[must_use]
pub fn refused_tool_ids(tool_calls: &Json) -> RefusedToolIdentifiers {
    let calls = match tool_calls {
        Json::Object(object) => object.get("tool_calls"),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Some(tool_calls)
        }
    };
    let Some(Json::List(items)) = calls else {
        return RefusedToolIdentifiers::default();
    };
    let mut identifiers = RefusedToolIdentifiers::default();
    for call in items {
        let Json::Object(fields) = call else {
            continue;
        };
        collect_identifiers(fields, &mut identifiers);
    }
    identifiers
}

fn collect_identifiers(fields: &IndexMap<String, Json>, identifiers: &mut RefusedToolIdentifiers) {
    for field in ["id", "call_id"] {
        if let Some(Json::Text(value)) = fields.get(field) {
            identifiers.ids.insert(value.clone());
        }
    }
    if let Some(Json::Text(name)) = fields.get("name") {
        identifiers.names.insert(name.clone());
    }
}

#[must_use]
pub fn is_refused_tool_use_block(
    block: &Json,
    refused_ids: &BTreeSet<String>,
    refused_names: &BTreeSet<String>,
) -> bool {
    let Json::Object(fields) = block else {
        return false;
    };
    let is_tool_use = fields
        .get("type")
        .is_some_and(|kind| matches!(kind, Json::Text(text) if text == "tool_use"));
    if !is_tool_use {
        return false;
    }
    match fields.get("id") {
        Some(Json::Text(block_id)) if !refused_ids.is_empty() => refused_ids.contains(block_id),
        _ => fields
            .get("name")
            .is_some_and(|name| matches!(name, Json::Text(text) if refused_names.contains(text))),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiSurface {
    AnthropicMessages,
    Responses,
}

#[must_use]
pub const fn can_finalize_turn(stream: bool, api_surface: ApiSurface) -> bool {
    match api_surface {
        ApiSurface::AnthropicMessages => !stream,
        ApiSurface::Responses => false,
    }
}
