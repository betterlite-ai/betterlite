use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::agentic_loop_ceiling::{
    AgenticLoopCeilingError, AgenticLoopValue, DEFAULT_MAX_AGENTIC_LOOPS,
    validated_max_agentic_loops,
};
use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use num_traits::ToPrimitive as _;
use serde_json::Number;

use super::error::{AgenticLoopSafetyError, AgenticLoopSettingsError};
use super::python_value::{python_repr, text_repr};

const CEILING_FIELD: &str = "litellm_params.max_agentic_loops";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgenticLoopSettings {
    pub depth: i64,
    pub max_loops: u32,
    pub fingerprints: Vec<Json>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefusedToolIdentifiers {
    pub ids: BTreeSet<String>,
    pub names: BTreeSet<String>,
}

pub fn get_agentic_loop_settings(
    kwargs: &IndexMap<String, Json>,
) -> Result<AgenticLoopSettings, AgenticLoopSettingsError> {
    let depth = trusted_depth(kwargs.get("_agentic_loop_depth"))?;
    let max_loops = match kwargs.get("max_agentic_loops") {
        Some(value) => agentic_ceiling(value)?,
        None => DEFAULT_MAX_AGENTIC_LOOPS,
    };
    let fingerprints = fingerprint_history(kwargs.get("_agentic_loop_fingerprints"))?;
    Ok(AgenticLoopSettings {
        depth,
        max_loops,
        fingerprints,
    })
}

fn agentic_ceiling(value: &Json) -> Result<u32, AgenticLoopSettingsError> {
    let parsed = match value {
        Json::Null => AgenticLoopValue::Null,
        Json::Bool(flag) => AgenticLoopValue::Boolean(*flag),
        Json::Number(number) => number_loop_value(number),
        Json::Text(text) => AgenticLoopValue::Text(text.clone()),
        // the shared validator has no list or object arm, and neither spells a ceiling
        Json::List(_) | Json::Object(_) => {
            return Err(AgenticLoopSettingsError::CeilingNotInteger {
                literal: python_repr(value),
            });
        }
    };
    match validated_max_agentic_loops(&parsed, CEILING_FIELD) {
        Ok(Some(ceiling)) => Ok(ceiling),
        Ok(None) => Ok(DEFAULT_MAX_AGENTIC_LOOPS),
        Err(AgenticLoopCeilingError::BelowMinimum { value: minimum, .. }) => {
            Err(AgenticLoopSettingsError::CeilingBelowMinimum { value: minimum })
        }
        Err(AgenticLoopCeilingError::NotWholeNumber { .. }) => {
            Err(AgenticLoopSettingsError::CeilingNotInteger {
                literal: python_repr(value),
            })
        }
    }
}

fn number_loop_value(number: &Number) -> AgenticLoopValue {
    let text = number.to_string();
    if text.contains(['.', 'e', 'E']) {
        return match text.parse::<f64>() {
            Ok(value) => AgenticLoopValue::Float(value),
            Err(_) => AgenticLoopValue::Text(text),
        };
    }
    match text.parse::<i64>() {
        Ok(value) => AgenticLoopValue::Integer(value),
        Err(_) => AgenticLoopValue::Text(text),
    }
}

fn trusted_depth(value: Option<&Json>) -> Result<i64, AgenticLoopSettingsError> {
    let Some(value) = value else {
        return Ok(0);
    };
    if is_falsy(value) {
        return Ok(0);
    }
    match value {
        Json::Bool(true) => Ok(1),
        Json::Number(number) => trusted_number_depth(number),
        Json::Text(text) => trusted_text_depth(text),
        Json::List(_) => Err(AgenticLoopSettingsError::DepthWrongType { type_name: "list" }),
        Json::Object(_) => Err(AgenticLoopSettingsError::DepthWrongType { type_name: "dict" }),
        // every remaining falsy value returned above; the arm keeps the match exhaustive
        Json::Null | Json::Bool(false) => Ok(0),
    }
}

fn is_falsy(value: &Json) -> bool {
    match value {
        Json::Null => true,
        Json::Bool(flag) => !*flag,
        Json::Number(number) => number_is_zero(number),
        Json::Text(text) => text.is_empty(),
        Json::List(items) => items.is_empty(),
        Json::Object(object) => object.is_empty(),
    }
}

fn number_is_zero(number: &Number) -> bool {
    let text = number.to_string();
    if text.contains(['.', 'e', 'E']) {
        let bits = number.as_f64().map(f64::to_bits);
        return matches!(bits, Some(0) | Some(NEGATIVE_ZERO_BITS));
    }
    text.parse::<i64>() == Ok(0)
}

const NEGATIVE_ZERO_BITS: u64 = 0x8000_0000_0000_0000;

fn trusted_number_depth(number: &Number) -> Result<i64, AgenticLoopSettingsError> {
    let text = number.to_string();
    if text.contains(['.', 'e', 'E']) {
        return float_depth(&text);
    }
    match text.parse::<i64>() {
        Ok(value) => Ok(value),
        Err(_) => Err(AgenticLoopSettingsError::DepthOutOfRange),
    }
}

fn float_depth(text: &str) -> Result<i64, AgenticLoopSettingsError> {
    let Ok(value) = text.parse::<f64>() else {
        return Err(AgenticLoopSettingsError::DepthOutOfRange);
    };
    if value.is_nan() {
        return Err(AgenticLoopSettingsError::DepthNan);
    }
    if value.is_infinite() {
        return Err(AgenticLoopSettingsError::DepthInfinity);
    }
    value
        .to_i64()
        .ok_or(AgenticLoopSettingsError::DepthOutOfRange)
}

fn trusted_text_depth(text: &str) -> Result<i64, AgenticLoopSettingsError> {
    let trimmed = text.trim();
    let digits = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    if !digits.split('_').all(|part| !part.is_empty()) {
        return Err(invalid_depth_literal(text));
    }
    match trimmed.replace('_', "").parse::<i64>() {
        Ok(value) => Ok(value),
        Err(_) => Err(invalid_depth_literal(text)),
    }
}

fn invalid_depth_literal(text: &str) -> AgenticLoopSettingsError {
    AgenticLoopSettingsError::DepthInvalidLiteral {
        literal: text_repr(text),
    }
}

fn fingerprint_history(value: Option<&Json>) -> Result<Vec<Json>, AgenticLoopSettingsError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if is_falsy(value) {
        return Ok(Vec::new());
    }
    match value {
        Json::List(items) => Ok(items.clone()),
        Json::Text(text) => Ok(text
            .chars()
            .map(|character| Json::Text(character.to_string()))
            .collect()),
        Json::Object(object) => Ok(object.keys().map(|key| Json::Text(key.clone())).collect()),
        Json::Bool(true) => {
            Err(AgenticLoopSettingsError::FingerprintsNotIterable { type_name: "bool" })
        }
        Json::Number(number) => Err(AgenticLoopSettingsError::FingerprintsNotIterable {
            type_name: number_type_name(number),
        }),
        // every remaining value is falsy and returned above
        Json::Null | Json::Bool(false) => Ok(Vec::new()),
    }
}

fn number_type_name(number: &Number) -> &'static str {
    let text = number.to_string();
    if text.contains(['.', 'e', 'E']) {
        "float"
    } else {
        "int"
    }
}

#[must_use]
pub fn fingerprint_agentic_tools(tools: &Json) -> String {
    let sorted = sort_json_keys(tools);
    python_json::dumps(&sorted)
}

fn sort_json_keys(value: &Json) -> Json {
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
    let fingerprint = fingerprint_agentic_tools(tool_calls);
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
pub fn refused_agentic_tool_identifiers(tool_calls: &Json) -> RefusedToolIdentifiers {
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

#[must_use]
pub const fn can_replace_turn_with_terminal_response(stream: bool, api_surface: &str) -> bool {
    match api_surface {
        "anthropic_messages" => !stream,
        _ => false,
    }
}
