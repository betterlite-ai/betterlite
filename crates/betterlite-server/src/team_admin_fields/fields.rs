use std::collections::BTreeMap;
use std::collections::BTreeSet;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;
use serde_json::Number;

use super::FOLDED_FIELDS;
use super::SYSTEM_MANAGED_KEYS;
use super::decoder::decode_team_admin_object;

pub(crate) type Fields = BTreeMap<String, JsonData>;

// LiteLLM compares the request and the stored row as JSON values, where an absent field and a null one match.
pub(crate) fn value_eq(left: Option<&JsonData>, right: Option<&JsonData>) -> bool {
    match (left, right) {
        (None | Some(JsonData::Null), None | Some(JsonData::Null)) => true,
        (Some(left_value), Some(right_value)) => json_eq(left_value, right_value),
        (None, Some(_)) | (Some(_), None) => false,
    }
}

fn json_eq(left: &JsonData, right: &JsonData) -> bool {
    match (left, right) {
        (JsonData::Null, JsonData::Null) => true,
        (JsonData::Bool(left_value), JsonData::Bool(right_value)) => left_value == right_value,
        (JsonData::Number(left_value), JsonData::Number(right_value)) => {
            number_eq(left_value, right_value)
        }
        (JsonData::String(left_value), JsonData::String(right_value)) => left_value == right_value,
        (JsonData::Array(left_items), JsonData::Array(right_items)) => {
            left_items.len() == right_items.len()
                && left_items
                    .iter()
                    .zip(right_items)
                    .all(|(left_item, right_item)| json_eq(left_item, right_item))
        }
        (JsonData::Object(left_fields), JsonData::Object(right_fields)) => {
            object_eq(left_fields, right_fields)
        }
        (
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::String(_)
            | JsonData::Array(_)
            | JsonData::Object(_),
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::String(_)
            | JsonData::Array(_)
            | JsonData::Object(_),
        ) => false,
    }
}

// Python numbers compare by value, so the arbitrary-precision text of a stored decimal meets a request float.
fn number_eq(left: &Number, right: &Number) -> bool {
    match (
        left.as_str().parse::<Decimal>(),
        right.as_str().parse::<Decimal>(),
    ) {
        (Ok(left_value), Ok(right_value)) => left_value == right_value,
        _ => left.as_str() == right.as_str(),
    }
}

pub(crate) fn object_eq(left: &Fields, right: &Fields) -> bool {
    left.len() == right.len()
        && left.iter().all(|(name, value)| {
            right
                .get(name)
                .is_some_and(|other| value_eq(Some(value), Some(other)))
        })
}

// `_as_object`: a stored JSON string is decoded with the bounded decoder, and anything else is no object.
pub(crate) fn as_object(value: Option<&JsonData>) -> Fields {
    match value {
        Some(JsonData::Object(fields)) => fields.clone(),
        Some(JsonData::String(text)) => decode_team_admin_object(text),
        Some(JsonData::Null | JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_))
        | None => Fields::new(),
    }
}

// `_metadata_changes`: the metadata as it would be stored against the stored one, key by key.
pub(crate) fn metadata_changes(supplied: &Fields, existing: &Fields) -> BTreeSet<String> {
    let stored = as_object(existing.get("metadata"));
    let mut merged = supplied
        .get("metadata")
        .map_or_else(|| stored.clone(), |value| as_object(Some(value)));
    for (name, value) in supplied {
        if FOLDED_FIELDS.contains(&name.as_str()) {
            merged.insert(name.clone(), value.clone());
        }
    }
    let mut keys: BTreeSet<&str> = merged.keys().map(String::as_str).collect();
    keys.extend(stored.keys().map(String::as_str));
    let mut changed = BTreeSet::new();
    for key in keys {
        if !SYSTEM_MANAGED_KEYS.contains(&key) && !value_eq(merged.get(key), stored.get(key)) {
            let name = if FOLDED_FIELDS.contains(&key) {
                key.to_owned()
            } else {
                "metadata".to_owned()
            };
            changed.insert(name);
        }
    }
    changed
}
