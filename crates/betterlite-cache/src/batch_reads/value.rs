use std::collections::BTreeMap;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::CachedValue;

// the Redis decoder reads a JSON float into the Object arm of CachedValue as this one-key map
const NUMBER_KEY: &str = "$serde_json::private::Number";

// both tiers read a stored null as a miss, exactly as InMemoryCache.get_cache does
pub(super) fn redis_value_json(value: &CachedValue) -> Option<Json> {
    match value {
        CachedValue::Null => None,
        stored => Some(json_from_cached(stored)),
    }
}

// every Redis answer a batch read gets back runs through here, whichever tier's map carries it
pub(super) fn redis_values<'value, Values>(values: Values) -> BTreeMap<String, Option<Json>>
where
    Values: IntoIterator<Item = (&'value String, &'value Option<CachedValue>)>,
{
    values
        .into_iter()
        .map(|(key, value)| (key.clone(), value.as_ref().and_then(redis_value_json)))
        .collect()
}

fn json_from_cached(value: &CachedValue) -> Json {
    match value {
        CachedValue::Null => Json::Null,
        CachedValue::Boolean(flag) => Json::Bool(*flag),
        CachedValue::Number(number) => Json::Number(number.clone()),
        CachedValue::Text(text) => Json::Text(text.clone()),
        CachedValue::Array(items) => Json::List(items.iter().map(json_from_cached).collect()),
        CachedValue::Object(fields) => json_from_object(fields),
    }
}

fn json_from_object(fields: &BTreeMap<String, CachedValue>) -> Json {
    if let Some(CachedValue::Text(digits)) = fields.get(NUMBER_KEY)
        && fields.len() == 1
        && let Ok(number) = digits.parse()
    {
        return Json::Number(number);
    }
    let converted = fields
        .iter()
        .map(|(key, entry)| (key.clone(), json_from_cached(entry)));
    Json::Object(converted.collect())
}

// mapped by hand as B4293 does: serde would round an integer past i64 range into a float
pub(super) fn cache_from_json(value: &Json) -> Option<CacheValue> {
    Some(match value {
        Json::Null => CacheValue::Null,
        Json::Bool(flag) => CacheValue::Boolean(*flag),
        Json::Number(number) => match (number.as_i64(), number.as_u64(), number.as_f64()) {
            (Some(whole), _, _) => CacheValue::Integer(whole),
            (None, None, Some(float)) => CacheValue::Float(float),
            _ => return None,
        },
        Json::Text(text) => CacheValue::Text(text.clone()),
        Json::List(items) => {
            CacheValue::Array(items.iter().map(cache_from_json).collect::<Option<_>>()?)
        }
        Json::Object(fields) => CacheValue::Object(
            fields
                .iter()
                .map(|(key, entry)| Some((key.clone(), cache_from_json(entry)?)))
                .collect::<Option<_>>()?,
        ),
    })
}
