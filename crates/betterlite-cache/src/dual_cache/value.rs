use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::CachedValue;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Number;

use super::DualCacheError;

// Json, CacheValue and CachedValue share one untagged JSON shape, so serde carries a value between tiers
pub(crate) fn convert<From, To>(value: &From) -> Result<To, DualCacheError>
where
    From: Serialize + ?Sized,
    To: DeserializeOwned,
{
    let wire =
        serde_json::to_value(value).map_err(|_source| DualCacheError::UnrepresentableValue)?;
    serde_json::from_value(wire).map_err(|_source| DualCacheError::UnrepresentableValue)
}

// mapped by hand: serde hands an arbitrary_precision float to CacheValue::Object; an integer past i64 would round as a float
pub(crate) fn cache_from_json(value: &Json) -> Result<CacheValue, DualCacheError> {
    match value {
        Json::Null => Ok(CacheValue::Null),
        Json::Bool(flag) => Ok(CacheValue::Boolean(*flag)),
        Json::Number(number) => {
            if let Some(whole) = number.as_i64() {
                return Ok(CacheValue::Integer(whole));
            }
            match number.as_f64() {
                Some(float) if number.as_u64().is_none() => Ok(CacheValue::Float(float)),
                _ => Err(DualCacheError::UnrepresentableValue),
            }
        }
        Json::Text(text) => Ok(CacheValue::Text(text.clone())),
        Json::List(items) => items
            .iter()
            .map(cache_from_json)
            .collect::<Result<_, _>>()
            .map(CacheValue::Array),
        Json::Object(fields) => fields
            .iter()
            .map(|(key, entry)| Ok((key.clone(), cache_from_json(entry)?)))
            .collect::<Result<_, _>>()
            .map(CacheValue::Object),
    }
}

pub(crate) fn json_from_cache(value: &CacheValue) -> Result<Json, DualCacheError> {
    match value {
        CacheValue::Credential(_) | CacheValue::Bytes(_) => {
            Err(DualCacheError::UnrepresentableValue)
        }
        CacheValue::Null => Ok(Json::Null),
        CacheValue::Boolean(flag) => Ok(Json::Bool(*flag)),
        CacheValue::Integer(whole) => Ok(Json::Number(Number::from(*whole))),
        CacheValue::Float(float) => Ok(Number::from_f64(*float).map_or(Json::Null, Json::Number)),
        CacheValue::Text(text) => Ok(Json::Text(text.clone())),
        CacheValue::Array(items) => items
            .iter()
            .map(json_from_cache)
            .collect::<Result<_, _>>()
            .map(Json::List),
        CacheValue::Object(fields) => fields
            .iter()
            .map(|(key, entry)| Ok((key.clone(), json_from_cache(entry)?)))
            .collect::<Result<_, _>>()
            .map(Json::Object),
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
