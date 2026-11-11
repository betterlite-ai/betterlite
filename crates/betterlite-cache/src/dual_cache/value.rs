use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::CachedValue;
use serde::Serialize;
use serde::de::DeserializeOwned;

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

// the memory tier holds integers as i64, and storing a wider one as a float would round it silently
pub(crate) fn cache_from_json(value: &Json) -> Result<CacheValue, DualCacheError> {
    if let Json::Number(number) = value
        && number.as_i64().is_none()
        && number.as_u64().is_some()
    {
        return Err(DualCacheError::UnrepresentableValue);
    }
    convert(value)
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
