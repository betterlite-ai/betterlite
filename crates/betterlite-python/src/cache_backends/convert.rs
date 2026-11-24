use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::CachedValue;
use serde::de::DeserializeOwned;
use serde_json::Number;

use super::error::BackendHandleError;

mod cached_value;
mod tests;

// the Redis decoder reads a JSON float into the Object arm of CachedValue as this one-key map
const NUMBER_KEY: &str = "$serde_json::private::Number";

pub(crate) fn wall_clock() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// ast-grep-ignore: no-one-line-functions - every backend's settings text takes this one parse
pub(crate) fn parse_settings<Settings>(text: &str) -> Result<Settings, BackendHandleError>
where
    Settings: DeserializeOwned,
{
    serde_json::from_str(text).map_err(|source| BackendHandleError::Settings { source })
}

// ast-grep-ignore: no-one-line-functions - a Python value arrives as JSON text, and one JSON cannot hold is a value error
pub(crate) fn json_value(text: &str) -> Result<Json, BackendHandleError> {
    serde_json::from_str(text).map_err(|source| BackendHandleError::Value { source })
}

// ast-grep-ignore: no-padded-one-line-functions - every write path reads its Python value through here
pub(crate) fn cache_value(text: &str) -> Result<CacheValue, BackendHandleError> {
    let json = json_value(text)?;
    Ok(cache_from_json(&json))
}

fn cache_from_json(value: &Json) -> CacheValue {
    match value {
        Json::Null => CacheValue::Null,
        Json::Bool(flag) => CacheValue::Boolean(*flag),
        Json::Number(number) => cache_number(number),
        Json::Text(text) => CacheValue::Text(text.clone()),
        Json::List(items) => CacheValue::Array(items.iter().map(cache_from_json).collect()),
        Json::Object(fields) => CacheValue::Object(
            fields
                .iter()
                .map(|(key, entry)| (key.clone(), cache_from_json(entry)))
                .collect(),
        ),
    }
}

// an integer keeps its exact form; anything else that is finite becomes a float
fn cache_number(number: &Number) -> CacheValue {
    match (number.as_i64(), number.as_f64()) {
        (Some(whole), _) => CacheValue::Integer(whole),
        (None, Some(float)) => CacheValue::Float(float),
        (None, None) => CacheValue::Text(number.to_string()),
    }
}

// ast-grep-ignore: no-one-line-functions - one encode error for every cached value
pub(crate) fn text_of_value(value: &CacheValue) -> Result<String, BackendHandleError> {
    serde_json::to_string(value).map_err(|source| BackendHandleError::Encode { source })
}

// ast-grep-ignore: no-one-line-functions - one encode error for every JSON answer
pub(crate) fn text_of_json(value: &Json) -> Result<String, BackendHandleError> {
    serde_json::to_string(value).map_err(|source| BackendHandleError::Encode { source })
}

// ast-grep-ignore: no-one-line-functions - the Redis decoder's value reaches JSON text through this rebuild
pub(crate) fn text_of_cached(value: CachedValue) -> Result<String, BackendHandleError> {
    text_of_json(&cached_value::json_from_cached(value))
}

pub(crate) fn number_text(value: f64) -> Result<String, BackendHandleError> {
    let number =
        Number::from_f64(value).ok_or(BackendHandleError::UnrepresentableNumber { value })?;
    text_of_json(&Json::Number(number))
}

// ast-grep-ignore: no-one-line-functions - a whole counter answer is JSON text through this one encode
pub(crate) fn whole_text(value: i64) -> Result<String, BackendHandleError> {
    text_of_json(&Json::Number(Number::from(value)))
}

// Redis wants whole seconds for its EXPIRE, and LiteLLM's int() truncates the fraction.
pub(crate) fn whole_seconds(value: f64) -> Result<i64, BackendHandleError> {
    let truncated = value.trunc();
    if !truncated.is_finite() {
        return Err(BackendHandleError::UnrepresentableNumber { value });
    }
    format!("{truncated:.0}")
        .parse()
        .map_err(|_source| BackendHandleError::NotWhole { value })
}

// DiskCache.increment_cache takes an int, so a fractional increment is the caller's error.
pub(crate) fn whole_increment(value: f64) -> Result<i64, BackendHandleError> {
    if value.fract().abs() > 0.0_f64 {
        return Err(BackendHandleError::NotWhole { value });
    }
    whole_seconds(value)
}
