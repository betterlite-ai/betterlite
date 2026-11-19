use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use super::error::{HitError, LookupError};

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
    serde_json::to_value(value)
        .and_then(serde_json::from_value)
        .unwrap_or(Json::Null)
}

// The stored value as the cache holds it: an entry list handed back to the caller keeps the shape it had before the read
pub fn cache_value_of<T: Serialize + ?Sized>(value: &T) -> Result<CacheValue, LookupError> {
    let rendered = serde_json::to_value(value).map_err(|source| LookupError::Render { source })?;
    serde_json::from_value(rendered).map_err(|source| LookupError::Render { source })
}

#[must_use]
pub fn kwargs_json(kwargs: &KeyObject) -> IndexMap<String, Json> {
    kwargs
        .iter()
        .map(|(name, value)| (name.clone(), key_value_json(value)))
        .collect()
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
