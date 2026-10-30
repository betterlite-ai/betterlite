use std::sync::Arc;

use betterlite_context::local_ttl_cache::{
    CacheError, CacheValue, LocalTtlCache, decode_cached_json,
};
use serde::Deserialize;
use serde_json::json;

#[path = "local_ttl_cache/tests.rs"]
mod tests;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    CacheDefaults,
    ExpiryAndOverwrite,
    NonrefreshingTtl,
    ValueSizePolicy,
    CachedJsonProjection { value: String },
    DualLocalDefaultsErrors,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::CacheDefaults => defaults(),
            Input::ExpiryAndOverwrite => expiry(),
            Input::NonrefreshingTtl => nonrefreshing(),
            Input::ValueSizePolicy => size_policy(),
            Input::CachedJsonProjection { value } => Ok(projection(value)),
            Input::DualLocalDefaultsErrors => dual_defaults(),
        }
    }
}

fn defaults() -> crate::helpers::HelperResult {
    let cache = LocalTtlCache::with_clock(None, None, None, Arc::new(|| 0.0_f64))?;
    Ok(json!({"capacity": cache.capacity(), "ttl": 600, "item_kib": cache.max_size_per_item()}))
}

fn expiry() -> crate::helpers::HelperResult {
    let mut cache =
        LocalTtlCache::with_clock(Some(2), Some(600.0_f64), Some(1024), Arc::new(|| 10.0_f64))?;
    cache.set("b", CacheValue::Integer(2), Some(5.0_f64))?;
    cache.set("a", CacheValue::Integer(1), Some(5.0_f64))?;
    cache.set("c", CacheValue::Integer(3), Some(5.0_f64))?;
    Ok(json!({"a": cache.get("a"), "b": cache.get("b"), "c": cache.get("c")}))
}

fn nonrefreshing() -> crate::helpers::HelperResult {
    let mut cache =
        LocalTtlCache::with_clock(Some(2), Some(600.0_f64), Some(1024), Arc::new(|| 0.0_f64))?;
    cache.set("key", CacheValue::Integer(1), Some(10.0_f64))?;
    cache.set("key", CacheValue::Integer(2), Some(100.0_f64))?;
    Ok(json!({"value": cache.get("key"), "expiry": cache.expiry("key")}))
}

fn size_policy() -> crate::helpers::HelperResult {
    let mut cache =
        LocalTtlCache::with_clock(Some(2), Some(600.0_f64), Some(1), Arc::new(|| 0.0_f64))?;
    cache.set("small", CacheValue::Text("ok".to_owned()), None)?;
    cache.set("large", CacheValue::Text("x".repeat(2000)), None)?;
    Ok(json!({"small": cache.get("small"), "large": cache.get("large")}))
}

fn projection(value: &str) -> serde_json::Value {
    let result = json!({"value": decode_cached_json(value.as_bytes())});
    result
}

fn dual_defaults() -> crate::helpers::HelperResult {
    let mut cache =
        LocalTtlCache::with_clock(Some(2), Some(600.0_f64), Some(1024), Arc::new(|| 0.0_f64))?;
    cache.set("false", CacheValue::Boolean(false), None)?;
    Ok(json!({"value": cache.get("false"), "missing": cache.get("missing")}))
}

impl From<CacheError> for Box<crate::helpers::HelperError> {
    fn from(error: CacheError) -> Self {
        Self::new(crate::helpers::HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: error.to_string(),
        })
    }
}
