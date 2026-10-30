use std::sync::Arc;

use betterlite_context::local_ttl_cache::{CacheError, LocalTtlCache, decode_cached_json};
use serde::Deserialize;
use serde_json::{Value, json};

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
            Input::CacheDefaults => {
                let cache = LocalTtlCache::with_clock(None, None, None, Arc::new(|| 0.0))?;
                Ok(
                    json!({"capacity": cache.capacity(), "ttl": cache.default_ttl(), "item_kib": cache.max_size_per_item()}),
                )
            }
            Input::ExpiryAndOverwrite => {
                let mut cache =
                    LocalTtlCache::with_clock(Some(2), Some(600.0), Some(1024), Arc::new(|| 10.0))?;
                cache.set("b", Value::from(2), Some(5.0))?;
                cache.set("a", Value::from(1), Some(5.0))?;
                cache.set("c", Value::from(3), Some(5.0))?;
                Ok(json!({"a": cache.get("a"), "b": cache.get("b"), "c": cache.get("c")}))
            }
            Input::NonrefreshingTtl => {
                let mut cache =
                    LocalTtlCache::with_clock(Some(2), Some(600.0), Some(1024), Arc::new(|| 0.0))?;
                cache.set("key", Value::from(1), Some(10.0))?;
                cache.set("key", Value::from(2), Some(100.0))?;
                Ok(json!({"value": cache.get("key"), "expiry": cache.expiry("key")}))
            }
            Input::ValueSizePolicy => {
                let mut cache =
                    LocalTtlCache::with_clock(Some(2), Some(600.0), Some(1), Arc::new(|| 0.0))?;
                cache.set("small", Value::from("ok"), None)?;
                cache.set("large", Value::from("x".repeat(2000)), None)?;
                Ok(json!({"small": cache.get("small"), "large": cache.get("large")}))
            }
            Input::CachedJsonProjection { value } => {
                Ok(json!({"value": decode_cached_json(value.as_bytes())}))
            }
            Input::DualLocalDefaultsErrors => {
                let mut cache =
                    LocalTtlCache::with_clock(Some(2), Some(600.0), Some(1024), Arc::new(|| 0.0))?;
                cache.set("false", Value::Bool(false), None)?;
                Ok(json!({"value": cache.get("false"), "missing": cache.get("missing")}))
            }
        }
    }
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
