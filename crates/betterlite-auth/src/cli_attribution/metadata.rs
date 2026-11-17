use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use super::claim_map::is_safe_claim_value;
use super::key_path::{insert_at, value_at};

// Nulls never land in the stored metadata, and two objects merge instead of one replacing the other.
pub fn merge_metadata(
    target: &mut BTreeMap<String, JsonData>,
    source: &BTreeMap<String, JsonData>,
) {
    let mut pending: Vec<(Vec<String>, &BTreeMap<String, JsonData>)> = vec![(Vec::new(), source)];
    while let Some((path, entries)) = pending.pop() {
        for (key, value) in entries {
            if matches!(value, JsonData::Null) {
                continue;
            }
            let mut key_path = path.clone();
            key_path.push(key.clone());
            if let JsonData::Object(nested) = value
                && matches!(value_at(target, &key_path), Some(JsonData::Object(_)))
            {
                pending.push((key_path, nested));
                continue;
            }
            insert_at(target, &key_path, value.clone());
        }
    }
}

#[must_use]
pub fn flatten_metadata_for_poll(
    metadata: &BTreeMap<String, JsonData>,
) -> BTreeMap<String, JsonData> {
    let mut flattened = BTreeMap::new();
    let mut pending: Vec<(String, &JsonData)> = metadata
        .iter()
        .map(|(key, value)| (key.clone(), value))
        .collect();
    while let Some((prefix, value)) = pending.pop() {
        if let JsonData::Object(children) = value {
            for (key, child) in children {
                pending.push((join_key_path(&prefix, key), child));
            }
            continue;
        }
        if is_safe_claim_value(value) {
            flattened.insert(prefix, value.clone());
        }
    }
    flattened
}

#[must_use]
pub fn poll_attribution_metadata(
    session: &BTreeMap<String, JsonData>,
) -> BTreeMap<String, JsonData> {
    let Some(JsonData::Object(stored)) = session.get("attribution_metadata") else {
        return BTreeMap::new();
    };
    flatten_metadata_for_poll(stored)
}

fn join_key_path(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        return key.to_owned();
    }
    format!("{prefix}.{key}")
}
