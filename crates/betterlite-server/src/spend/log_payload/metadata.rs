use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;

const OBJECT_KEY_FIELD: &str = "cold_storage_object_key";

// json.loads over stored metadata text: a JSON scalar stays a scalar, and only text that is not JSON becomes {}
#[must_use]
pub fn decode_log_metadata(bytes: &[u8]) -> JsonData {
    serde_json::from_slice::<Json>(bytes)
        .map_or_else(|_malformed| JsonData::Object(BTreeMap::new()), row_json)
}

// decoded JSON back in the row type; a repeated key already kept its last value, as json.loads does
#[must_use]
pub fn row_json(value: Json) -> JsonData {
    match value {
        Json::Null => JsonData::Null,
        Json::Bool(flag) => JsonData::Bool(flag),
        Json::Number(number) => JsonData::Number(number),
        Json::Text(text) => JsonData::String(text),
        Json::List(items) => JsonData::Array(items.into_iter().map(row_json).collect()),
        Json::Object(fields) => JsonData::Object(
            fields
                .into_iter()
                .map(|(key, item)| (key, row_json(item)))
                .collect::<BTreeMap<_, _>>(),
        ),
    }
}

// _hydrate_spend_log_metadata for one row: only string metadata is decoded, anything else stays as read
pub fn hydrate_log_metadata(metadata: &mut JsonData) {
    if let JsonData::String(text) = metadata {
        let decoded = decode_log_metadata(text.as_bytes());
        *metadata = decoded;
    }
}

// _cold_storage_object_key_from_metadata: metadata text is decoded first, and only an object can name a key
#[must_use]
pub fn cold_storage_object_key(metadata: Option<&JsonData>) -> Option<String> {
    match metadata? {
        JsonData::String(text) => match decode_log_metadata(text.as_bytes()) {
            JsonData::Object(fields) => object_key(&fields),
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::String(_)
            | JsonData::Array(_) => None,
        },
        JsonData::Object(fields) => object_key(fields),
        JsonData::Null | JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) => None,
    }
}

// only a nonempty string under the key names a stored object
fn object_key(fields: &BTreeMap<String, JsonData>) -> Option<String> {
    match fields.get(OBJECT_KEY_FIELD) {
        Some(JsonData::String(key)) if !key.is_empty() => Some(key.clone()),
        Some(_) | None => None,
    }
}
