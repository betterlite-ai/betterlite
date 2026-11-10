use super::support::Fields;
use crate::spend_sanitize::value::SpendValue;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
pub const CLASSIFIER_AUDIT_FIELDS: [&str; 2] = ["classifier_input", "originating_request_masked"];
#[derive(Clone, Copy)]
pub enum SnapshotValue<'data> {
    Text(&'data str),
    Bytes(&'data [u8]),
    Value(&'data SpendValue),
}
#[must_use]
pub fn classifier_input_snapshot(value: SnapshotValue<'_>, openai_sdk: bool) -> Option<Json> {
    match value {
        SnapshotValue::Text(text) => decode_classifier_snapshot(text.as_bytes()),
        SnapshotValue::Bytes(bytes) => decode_classifier_snapshot(bytes),
        SnapshotValue::Value(content) => {
            if let SpendValue::Text(text) = content {
                return decode_classifier_snapshot(text.as_bytes());
            }
            value_object(content, openai_sdk)
        }
    }
}
#[must_use]
pub fn decode_classifier_snapshot(bytes: &[u8]) -> Option<Json> {
    let parsed: Json = serde_json::from_slice(bytes).ok()?;
    if matches!(parsed, Json::Object(_)) {
        return Some(parsed);
    }
    None
}
fn value_object(value: &SpendValue, openai_sdk: bool) -> Option<Json> {
    let SpendValue::Mapping(mapping) = value else {
        return None;
    };
    if !openai_sdk {
        return mapping_object(&mapping.entries);
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
    mapping_object(&body)
}
// pydantic's dict[str, JsonValue] rebuilds containers and rejects a value that is not JSON-native
fn mapping_object(entries: &IndexMap<String, SpendValue>) -> Option<Json> {
    let rebuilt = entries
        .iter()
        .map(|(key, item)| json_compatible(item).map(|converted| (key.clone(), converted)))
        .collect::<Option<IndexMap<_, _>>>()?;
    Some(Json::Object(rebuilt))
}
fn json_compatible(value: &SpendValue) -> Option<Json> {
    match value {
        SpendValue::Null => Some(Json::Null),
        SpendValue::Bool(flag) => Some(Json::Bool(*flag)),
        SpendValue::Number(number) => Some(Json::Number(number.clone())),
        SpendValue::Text(text) => Some(Json::Text(text.clone())),
        SpendValue::List(list) => Some(Json::List(
            list.items
                .iter()
                .map(json_compatible)
                .collect::<Option<Vec<_>>>()?,
        )),
        SpendValue::Mapping(mapping) => mapping_object(&mapping.entries),
        SpendValue::Model(_) | SpendValue::Object(_) => None,
    }
}
#[must_use]
pub fn classifier_audit_fields(payload: &Fields) -> Fields {
    CLASSIFIER_AUDIT_FIELDS
        .iter()
        .filter_map(|key| {
            let snapshot = payload.get(*key).and_then(json_snapshot)?;
            Some((key.to_string(), snapshot))
        })
        .collect()
}
// a JSON value is already JSON-native, so only a string must parse and only an object is accepted
fn json_snapshot(value: &Json) -> Option<Json> {
    match value {
        Json::Text(text) => decode_classifier_snapshot(text.as_bytes()),
        Json::Object(_) => Some(value.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => None,
    }
}
#[must_use]
pub fn without_classifier_audit(payload: &Fields) -> Fields {
    payload
        .iter()
        .filter(|(key, _)| !CLASSIFIER_AUDIT_FIELDS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
