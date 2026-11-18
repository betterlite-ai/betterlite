use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::types::embedding::CachedEmbedding;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::error::HitError;

#[must_use]
pub fn key_value_json(value: &KeyValue) -> Json {
    match value {
        KeyValue::Null => Json::Null,
        KeyValue::Bool(flag) => Json::Bool(*flag),
        KeyValue::Number(number) => Json::Number(number.clone()),
        KeyValue::String(text) => Json::Text(text.clone()),
        KeyValue::Array(items) => Json::List(items.iter().map(key_value_json).collect()),
        KeyValue::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(name, entry)| (name.clone(), key_value_json(entry)))
                .collect(),
        ),
    }
}

// The cached value as the logging readers take it; a credential handle or raw bytes never round-trip through a response cache
#[must_use]
pub fn cache_value_json(value: &CacheValue) -> Json {
    match value {
        CacheValue::Null => Json::Null,
        CacheValue::Boolean(flag) => Json::Bool(*flag),
        CacheValue::Integer(number) => Json::Number(Number::from(*number)),
        CacheValue::Float(number) => Number::from_f64(*number).map_or(Json::Null, Json::Number),
        CacheValue::Text(text) => Json::Text(text.clone()),
        CacheValue::Array(items) => Json::List(items.iter().map(cache_value_json).collect()),
        CacheValue::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(name, entry)| (name.clone(), cache_value_json(entry)))
                .collect(),
        ),
        CacheValue::Credential(_) | CacheValue::Bytes(_) => Json::Null,
    }
}

#[must_use]
pub fn kwargs_json(kwargs: &KeyObject) -> IndexMap<String, Json> {
    kwargs
        .iter()
        .map(|(name, value)| (name.clone(), key_value_json(value)))
        .collect()
}

// A raw per-input embedding list stays a JSON list when it is handed back unconverted
pub fn embedding_entries_cache_value(
    entries: &[Option<CachedEmbedding>],
) -> Result<CacheValue, serde_json::Error> {
    let value = serde_json::to_value(entries)?;
    serde_json::from_value(value)
}

// The request field as the resolution and the cache key read it: absent and null are unset, any other type is refused
pub fn request_text<'kwargs>(
    kwargs: &'kwargs KeyObject,
    field: &'static str,
) -> Result<Option<&'kwargs str>, HitError> {
    match kwargs.get(field) {
        None | Some(KeyValue::Null) => Ok(None),
        Some(KeyValue::String(text)) => Ok(Some(text)),
        Some(_) => Err(HitError::FieldType {
            field,
            expected: "a string",
        }),
    }
}
