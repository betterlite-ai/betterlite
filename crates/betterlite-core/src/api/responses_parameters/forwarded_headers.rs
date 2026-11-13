use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::controls::json_value::JsonValue;

#[must_use]
pub fn merge_forwarded_headers(
    extra_headers: Option<BTreeMap<String, JsonValue>>,
    client_headers: Option<BTreeMap<String, String>>,
) -> Option<BTreeMap<String, JsonValue>> {
    let Some(client) = client_headers.filter(|headers| !headers.is_empty()) else {
        return extra_headers;
    };
    let client_as_values = client
        .into_iter()
        .map(|(name, value)| (name, JsonValue::Text(value)));
    let Some(explicit) = extra_headers.filter(|headers| !headers.is_empty()) else {
        return Some(client_as_values.collect());
    };
    let explicit_names: BTreeSet<String> =
        explicit.keys().map(|name| name.to_lowercase()).collect();
    let mut merged: BTreeMap<String, JsonValue> = client_as_values
        .filter(|(name, _value)| !explicit_names.contains(&name.to_lowercase()))
        .collect();
    merged.extend(explicit);
    Some(merged)
}
