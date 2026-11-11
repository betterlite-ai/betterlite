use std::collections::BTreeMap;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::CachedValue;
use serde_json::Number;

use super::DualCacheError;

pub(crate) fn cache_from_json(value: &Json) -> Result<CacheValue, DualCacheError> {
    match value {
        Json::Null => Ok(CacheValue::Null),
        Json::Bool(flag) => Ok(CacheValue::Boolean(*flag)),
        Json::Number(number) => number_to_cache(number),
        Json::Text(text) => Ok(CacheValue::Text(text.clone())),
        Json::List(items) => items
            .iter()
            .map(cache_from_json)
            .collect::<Result<Vec<_>, _>>()
            .map(CacheValue::Array),
        Json::Object(fields) => fields
            .iter()
            .map(|(key, entry)| cache_from_json(entry).map(|converted| (key.clone(), converted)))
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map(CacheValue::Object),
    }
}

fn number_to_cache(number: &Number) -> Result<CacheValue, DualCacheError> {
    if let Some(integer) = number.as_i64() {
        return Ok(CacheValue::Integer(integer));
    }
    if let Some(unsigned) = number.as_u64() {
        let integer = i64::try_from(unsigned)
            .map_err(|_source| DualCacheError::IntegerOutOfRange { value: unsigned })?;
        return Ok(CacheValue::Integer(integer));
    }
    let float = number
        .as_f64()
        .ok_or(DualCacheError::UnrepresentableValue)?;
    Ok(CacheValue::Float(float))
}

pub(crate) fn json_from_cache(value: &CacheValue) -> Result<Json, DualCacheError> {
    match value {
        CacheValue::Null => Ok(Json::Null),
        CacheValue::Boolean(flag) => Ok(Json::Bool(*flag)),
        CacheValue::Integer(integer) => Ok(Json::Number(Number::from(*integer))),
        CacheValue::Float(float) => Number::from_f64(*float)
            .map(Json::Number)
            .ok_or(DualCacheError::NonFiniteNumber { value: *float }),
        CacheValue::Text(text) => Ok(Json::Text(text.clone())),
        CacheValue::Array(items) => items
            .iter()
            .map(json_from_cache)
            .collect::<Result<Vec<_>, _>>()
            .map(Json::List),
        CacheValue::Object(fields) => fields
            .iter()
            .map(|(key, entry)| json_from_cache(entry).map(|converted| (key.clone(), converted)))
            .collect::<Result<_, _>>()
            .map(Json::Object),
        CacheValue::Bytes(_) | CacheValue::Credential(_) => {
            Err(DualCacheError::UnrepresentableValue)
        }
    }
}

pub(crate) fn json_from_cached(value: CachedValue) -> Json {
    match value {
        CachedValue::Null => Json::Null,
        CachedValue::Boolean(flag) => Json::Bool(flag),
        CachedValue::Number(number) => Json::Number(number),
        CachedValue::Text(text) => Json::Text(text),
        CachedValue::Array(items) => Json::List(items.into_iter().map(json_from_cached).collect()),
        CachedValue::Object(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(key, entry)| (key, json_from_cached(entry)))
                .collect(),
        ),
    }
}
