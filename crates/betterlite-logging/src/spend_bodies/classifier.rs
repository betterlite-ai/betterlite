use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support::{NodeAllocator, spend_value};
use crate::spend_sanitize::value::{SpendList, SpendMapping, SpendValue};

pub const CLASSIFIER_AUDIT_FIELDS: [&str; 2] = ["classifier_input", "originating_request_masked"];

#[derive(Clone, Copy)]
pub enum SnapshotValue<'data> {
    Text(&'data str),
    Bytes(&'data [u8]),
    Value(&'data SpendValue),
}

#[must_use]
pub fn classifier_input_snapshot(
    value: SnapshotValue<'_>,
    openai_sdk: bool,
    nodes: &mut NodeAllocator,
) -> Option<SpendMapping> {
    match value {
        SnapshotValue::Text(text) => json_object(text.as_bytes(), nodes),
        SnapshotValue::Bytes(bytes) => json_object(bytes, nodes),
        SnapshotValue::Value(content) => match content {
            SpendValue::Text(text) => json_object(text.as_bytes(), nodes),
            SpendValue::Null
            | SpendValue::Bool(_)
            | SpendValue::Number(_)
            | SpendValue::List(_)
            | SpendValue::Mapping(_)
            | SpendValue::Model(_)
            | SpendValue::Object(_) => value_object(content, openai_sdk, nodes),
        },
    }
}

#[must_use]
pub fn decode_classifier_snapshot(bytes: &[u8]) -> Option<SpendMapping> {
    let parsed: Json = serde_json::from_slice(bytes).ok()?;
    let Json::Object(fields) = parsed else {
        return None;
    };
    let mut nodes = NodeAllocator::default();
    let mut decoded = IndexMap::with_capacity(fields.len());
    for (key, item) in &fields {
        decoded.insert(key.clone(), spend_value(item, &mut nodes));
    }
    validated(decoded, &mut nodes)
}

fn json_object(bytes: &[u8], nodes: &mut NodeAllocator) -> Option<SpendMapping> {
    let parsed: Json = serde_json::from_slice(bytes).ok()?;
    let Json::Object(fields) = parsed else {
        return None;
    };
    let mut decoded = IndexMap::with_capacity(fields.len());
    for (key, item) in &fields {
        decoded.insert(key.clone(), spend_value(item, nodes));
    }
    validated(decoded, nodes)
}

fn value_object(
    value: &SpendValue,
    openai_sdk: bool,
    nodes: &mut NodeAllocator,
) -> Option<SpendMapping> {
    let SpendValue::Mapping(mapping) = value else {
        return None;
    };
    if !openai_sdk {
        return validated(mapping.entries.clone(), nodes);
    }
    let mut body = IndexMap::with_capacity(mapping.entries.len());
    for (key, item) in &mapping.entries {
        if !matches!(key.as_str(), "extra_headers" | "extra_query" | "extra_body") {
            body.insert(key.clone(), item.clone());
        }
    }
    if let Some(SpendValue::Mapping(extra_body)) = mapping.entries.get("extra_body") {
        for (key, item) in &extra_body.entries {
            body.insert(key.clone(), item.clone());
        }
    }
    validated(body, nodes)
}

// pydantic's dict[str, JsonValue] rebuilds every container and rejects any value that is not JSON-native
fn validated(
    entries: IndexMap<String, SpendValue>,
    nodes: &mut NodeAllocator,
) -> Option<SpendMapping> {
    let mut rebuilt = IndexMap::with_capacity(entries.len());
    for (key, value) in entries {
        rebuilt.insert(key, json_compatible(&value, nodes)?);
    }
    Some(SpendMapping {
        node: nodes.allocate(),
        entries: rebuilt,
    })
}

fn json_compatible(value: &SpendValue, nodes: &mut NodeAllocator) -> Option<SpendValue> {
    match value {
        SpendValue::Null | SpendValue::Bool(_) | SpendValue::Number(_) | SpendValue::Text(_) => {
            Some(value.clone())
        }
        SpendValue::List(list) => {
            let mut items = Vec::with_capacity(list.items.len());
            for item in &list.items {
                items.push(json_compatible(item, nodes)?);
            }
            Some(SpendValue::List(SpendList {
                node: nodes.allocate(),
                items,
            }))
        }
        SpendValue::Mapping(mapping) => {
            validated(mapping.entries.clone(), nodes).map(SpendValue::Mapping)
        }
        SpendValue::Model(_) | SpendValue::Object(_) => None,
    }
}

#[must_use]
pub fn classifier_audit_fields(
    payload: &SpendMapping,
    nodes: &mut NodeAllocator,
) -> IndexMap<String, SpendValue> {
    let classifier_input = payload
        .entries
        .get("classifier_input")
        .and_then(|value| classifier_input_snapshot(SnapshotValue::Value(value), false, nodes));
    let originating = payload
        .entries
        .get("originating_request_masked")
        .and_then(|value| classifier_input_snapshot(SnapshotValue::Value(value), false, nodes));
    match (classifier_input, originating) {
        (Some(classifier), Some(originating_request)) => IndexMap::from([
            (
                "classifier_input".to_owned(),
                SpendValue::Mapping(classifier),
            ),
            (
                "originating_request_masked".to_owned(),
                SpendValue::Mapping(originating_request),
            ),
        ]),
        (Some(classifier), None) => IndexMap::from([(
            "classifier_input".to_owned(),
            SpendValue::Mapping(classifier),
        )]),
        (None, Some(originating_request)) => IndexMap::from([(
            "originating_request_masked".to_owned(),
            SpendValue::Mapping(originating_request),
        )]),
        (None, None) => IndexMap::new(),
    }
}

#[must_use]
pub fn without_classifier_audit(payload: &SpendMapping, nodes: &mut NodeAllocator) -> SpendMapping {
    let mut entries = IndexMap::with_capacity(payload.entries.len());
    for (key, value) in &payload.entries {
        if CLASSIFIER_AUDIT_FIELDS.contains(&key.as_str()) {
            continue;
        }
        entries.insert(key.clone(), value.clone());
    }
    SpendMapping {
        node: nodes.allocate(),
        entries,
    }
}
