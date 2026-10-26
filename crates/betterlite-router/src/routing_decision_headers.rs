use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

mod python_text;

const COMPLEXITY_PREFIX: &str = "x-litellm-complexity-router-";
const QUALITY_HEADERS: [(&str, &str); 5] = [
    ("routed_model", "x-litellm-quality-router-model"),
    ("quality_tier", "x-litellm-quality-router-tier"),
    ("routed_via", "x-litellm-quality-router-via"),
    ("matched_keyword", "x-litellm-quality-router-keyword"),
    ("complexity_tier", "x-litellm-quality-router-complexity"),
];

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum RoutingValue {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    // Python's str() prints a dict in insertion order
    Object(IndexMap<String, Self>),
}

#[must_use]
pub fn complexity_headers(request: &RoutingValue) -> BTreeMap<String, String> {
    let Some(data) = object(request) else {
        return BTreeMap::new();
    };
    let metadata_key = if data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    };
    let Some(metadata) = data.get(metadata_key).and_then(object) else {
        return BTreeMap::new();
    };
    let Some(decision) = metadata.get("routing_decision").and_then(object) else {
        return BTreeMap::new();
    };
    if decision.get("router_type") != Some(&RoutingValue::String("complexity".to_owned())) {
        return BTreeMap::new();
    }
    let score_value = decision
        .get("score")
        .and_then(score_text)
        .map(RoutingValue::String);
    let reasoning_effort = decision
        .get("tier_litellm_params")
        .and_then(object)
        .and_then(|params| params.get("reasoning_effort"));
    let fields = [
        ("tier", decision.get("tier")),
        ("cause", decision.get("cause")),
        ("score", score_value.as_ref()),
        ("reasoning-effort", reasoning_effort),
    ];
    fields
        .into_iter()
        .filter_map(|(name, raw_value)| {
            let header_value = raw_value.and_then(header_string)?;
            Some((format!("{COMPLEXITY_PREFIX}{name}"), header_value))
        })
        .collect()
}

#[must_use]
pub fn replace_complexity_headers(
    existing: &BTreeMap<String, RoutingValue>,
    new_headers: &BTreeMap<String, String>,
) -> BTreeMap<String, RoutingValue> {
    let mut result = existing
        .iter()
        .filter(|(key, _)| new_headers.contains_key(*key) || !key.starts_with(COMPLEXITY_PREFIX))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    result.extend(
        new_headers
            .iter()
            .map(|(key, value)| (key.clone(), RoutingValue::String(value.clone()))),
    );
    result
}

pub fn apply_quality_headers(
    additional_headers: &mut BTreeMap<String, RoutingValue>,
    request: &RoutingValue,
) {
    let Some(data) = object(request) else {
        return;
    };
    let Some(decision) = data
        .get("metadata")
        .and_then(object)
        .and_then(|metadata| metadata.get("quality_router_decision"))
        .and_then(object)
    else {
        return;
    };
    for (field, header) in QUALITY_HEADERS {
        let Some(value) = decision
            .get(field)
            .filter(|value| !matches!(value, RoutingValue::Null))
        else {
            continue;
        };
        additional_headers.insert(
            header.to_owned(),
            RoutingValue::String(python_text::python_str(value)),
        );
    }
}

#[must_use]
pub fn decode_routing_header_value(value: &str) -> Option<String> {
    let normalized = value.trim();
    (!normalized.is_empty() && normalized.bytes().all(|byte| (b' '..=b'~').contains(&byte)))
        .then(|| normalized.to_owned())
}

const fn object(value: &RoutingValue) -> Option<&IndexMap<String, RoutingValue>> {
    match value {
        RoutingValue::Object(fields) => Some(fields),
        RoutingValue::Null
        | RoutingValue::Bool(_)
        | RoutingValue::Number(_)
        | RoutingValue::String(_)
        | RoutingValue::Array(_) => None,
    }
}

// serde_json::Number never holds NaN or infinity, so every number is a finite score
fn score_text(value: &RoutingValue) -> Option<String> {
    match value {
        RoutingValue::Number(number) => Some(python_text::python_number(number)),
        RoutingValue::Null
        | RoutingValue::Bool(_)
        | RoutingValue::String(_)
        | RoutingValue::Array(_)
        | RoutingValue::Object(_) => None,
    }
}

fn header_string(value: &RoutingValue) -> Option<String> {
    match value {
        RoutingValue::String(text) => decode_routing_header_value(text),
        RoutingValue::Null
        | RoutingValue::Bool(_)
        | RoutingValue::Number(_)
        | RoutingValue::Array(_)
        | RoutingValue::Object(_) => None,
    }
}

mod tests;
