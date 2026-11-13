use std::collections::BTreeMap;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::CachedValue;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::DualCacheError;

// the Redis decoder reads a JSON float into the Object arm of CachedValue as this one-key map
const NUMBER_KEY: &str = "$serde_json::private::Number";

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

// mapped by hand: serde hands an arbitrary_precision float to CacheValue::Object, and an integer past i64 would round as a float
pub(crate) fn cache_from_json(value: &Json) -> Result<CacheValue, DualCacheError> {
    Ok(match value {
        Json::Null => CacheValue::Null,
        Json::Bool(flag) => CacheValue::Boolean(*flag),
        Json::Number(number) => match (number.as_i64(), number.as_u64(), number.as_f64()) {
            (Some(whole), _, _) => CacheValue::Integer(whole),
            (None, None, Some(float)) => CacheValue::Float(float),
            _ => return Err(DualCacheError::UnrepresentableValue),
        },
        Json::Text(text) => CacheValue::Text(text.clone()),
        Json::List(items) => CacheValue::Array(
            items
                .iter()
                .map(cache_from_json)
                .collect::<Result<_, DualCacheError>>()?,
        ),
        Json::Object(fields) => CacheValue::Object(
            fields
                .iter()
                .map(|(key, entry)| Ok((key.clone(), cache_from_json(entry)?)))
                .collect::<Result<_, DualCacheError>>()?,
        ),
    })
}

pub(crate) fn json_from_cached(value: CachedValue) -> Json {
    match value {
        CachedValue::Null => Json::Null,
        CachedValue::Boolean(flag) => Json::Bool(flag),
        CachedValue::Number(number) => Json::Number(number),
        CachedValue::Text(text) => Json::Text(text),
        CachedValue::Array(items) => Json::List(items.into_iter().map(json_from_cached).collect()),
        CachedValue::Object(fields) => json_from_object(fields),
    }
}

fn json_from_object(fields: BTreeMap<String, CachedValue>) -> Json {
    if let Some(CachedValue::Text(digits)) = fields.get(NUMBER_KEY)
        && fields.len() == 1
        && let Ok(number) = digits.parse()
    {
        return Json::Number(number);
    }
    let converted = fields
        .into_iter()
        .map(|(key, entry)| (key, json_from_cached(entry)));
    Json::Object(converted.collect())
}
