use std::collections::HashMap;

use betterlite_core::api::request_guardrail_policy::GuardrailMetadata;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

const WILDCARD: &str = "[*]";
const WILDCARD_AFTER_DOT: &str = ".[*]";

// pydantic's default config ignores an extra key, so this record does too
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PassthroughGuardrailSettings {
    pub request_fields: Option<Vec<String>>,
    pub response_fields: Option<Vec<String>>,
}

// LiteLLM's normalize_config keeps each settings object as sent; only get_settings builds the model
pub type PassthroughGuardrailsConfig = IndexMap<String, Option<Json>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonPathValue {
    Single(String),
    Many(Vec<String>),
}

#[must_use]
pub fn extract_fields(data: &Json, expressions: &[String]) -> String {
    let mut extracted: Vec<String> = Vec::new();
    for expression in expressions {
        match evaluate(data, expression) {
            None => tracing::debug!("failed to extract jsonpath field {expression}"),
            Some(JsonPathValue::Single(value)) => {
                if !value.is_empty() {
                    extracted.push(value);
                }
            }
            Some(JsonPathValue::Many(values)) => {
                extracted.extend(values.into_iter().filter(|value| !value.is_empty()));
            }
        }
    }
    extracted.join("\n")
}

#[must_use]
pub fn evaluate(data: &Json, expression: &str) -> Option<JsonPathValue> {
    if expression.is_empty() {
        return None;
    }
    let normalized = expression.replace(WILDCARD, WILDCARD_AFTER_DOT);
    let parts: Vec<&str> = normalized.split('.').collect();
    let mut current = data;
    for (index, part) in parts.iter().enumerate() {
        if matches!(current, Json::Null) {
            return None;
        }
        if *part == WILDCARD {
            return wildcard_value(current, &parts, index);
        }
        let Json::Object(object) = current else {
            return None;
        };
        current = object.get(*part)?;
    }
    leaf_value(current)
}

fn wildcard_value(current: &Json, parts: &[&str], index: usize) -> Option<JsonPathValue> {
    let Json::List(items) = current else {
        return None;
    };
    let remaining = parts
        .iter()
        .skip(index.saturating_add(1))
        .copied()
        .collect::<Vec<&str>>()
        .join(".");
    if remaining.is_empty() {
        return leaf_value(current);
    }
    let mut results: Vec<String> = Vec::new();
    for item in items {
        let Json::Object(_) = item else {
            continue;
        };
        let Some(value) = evaluate(item, &remaining) else {
            continue;
        };
        match value {
            JsonPathValue::Single(text) => results.push(text),
            JsonPathValue::Many(texts) => results.extend(texts),
        }
    }
    (!results.is_empty()).then_some(JsonPathValue::Many(results))
}

fn leaf_value(value: &Json) -> Option<JsonPathValue> {
    if let Json::List(items) = value {
        let mut texts: Vec<String> = Vec::with_capacity(items.len());
        for item in items {
            let Some(text) = item_text(item) else {
                continue;
            };
            texts.push(text);
        }
        return Some(JsonPathValue::Many(texts));
    }
    item_text(value).map(JsonPathValue::Single)
}

// a string takes no quotes; every other value reads as standard JSON, since Python's str text is not ported
fn item_text(value: &Json) -> Option<String> {
    match value {
        Json::Null => None,
        Json::Text(text) => Some(text.clone()),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            serde_json::to_string(value).ok()
        }
    }
}

#[must_use]
pub fn normalize_config(config: Option<&Json>) -> Option<PassthroughGuardrailsConfig> {
    match config? {
        Json::Null => None,
        Json::List(names) => normalized_names(names),
        Json::Object(entries) => Some(normalized_settings(entries)),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) => {
            tracing::debug!("passthrough guardrails config is not a list or an object");
            None
        }
    }
}

fn normalized_names(names: &[Json]) -> Option<PassthroughGuardrailsConfig> {
    let mut config = PassthroughGuardrailsConfig::new();
    for name in names {
        let Json::Text(field) = name else {
            tracing::debug!("a passthrough guardrail name is not a string");
            return None;
        };
        config.insert(field.clone(), None);
    }
    Some(config)
}

fn normalized_settings(entries: &IndexMap<String, Json>) -> PassthroughGuardrailsConfig {
    let mut config = PassthroughGuardrailsConfig::new();
    for (name, value) in entries {
        let settings = if matches!(value, Json::Null) {
            None
        } else {
            Some(value.clone())
        };
        config.insert(name.clone(), settings);
    }
    config
}

#[must_use]
pub fn get_settings(config: Option<&Json>, name: &str) -> Option<PassthroughGuardrailSettings> {
    let normalized = normalize_config(config)?;
    let settings = normalized.get(name)?;
    let value = settings.as_ref()?;
    let serialized = serde_json::to_value(value).ok()?;
    serde_json::from_value::<PassthroughGuardrailSettings>(serialized).ok()
}

#[must_use]
pub fn collect_guardrails(
    key_metadata: &GuardrailMetadata,
    team_metadata: &GuardrailMetadata,
    config: Option<&Json>,
) -> Option<HashMap<String, bool>> {
    let normalized = normalize_config(config)?;
    if normalized.is_empty() {
        tracing::debug!("passthrough guardrails config is empty");
        return None;
    }
    let mut collected: HashMap<String, bool> =
        normalized.keys().map(|name| (name.clone(), true)).collect();
    // the inherited merge keeps key before team and the first occurrence of a repeated name
    for name in key_metadata
        .guardrails
        .iter()
        .chain(team_metadata.guardrails.iter())
    {
        collected.entry(name.clone()).or_insert(true);
    }
    Some(collected)
}

mod tests;
