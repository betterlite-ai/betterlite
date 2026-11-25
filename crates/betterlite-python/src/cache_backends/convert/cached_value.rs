use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::CachedValue;
use serde_json::Number;

use super::NUMBER_KEY;

// CachedValue mirrors JSON but has no Serialize impl, so the answer is rebuilt as Json first
pub(super) fn json_from_cached(value: CachedValue) -> Json {
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
    if fields.len() == 1_usize
        && let Some(CachedValue::Text(digits)) = fields.get(NUMBER_KEY)
        && let Ok(number) = digits.parse::<Number>()
    {
        return Json::Number(number);
    }
    let converted = fields
        .into_iter()
        .map(|(key, entry)| (key, json_from_cached(entry)));
    Json::Object(converted.collect())
}
