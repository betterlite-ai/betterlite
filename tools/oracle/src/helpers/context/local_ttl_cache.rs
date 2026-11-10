use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
#[path = "local_ttl_cache/dual.rs"]
mod dual;

use betterlite_context::local_ttl_cache::{CacheError, CacheValue, LocalTtlCache};
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
            Input::ValueSizePolicy => {
                let mut cache = LocalTtlCache::with_clock(
                    Some(2),
                    Some(600.0_f64),
                    Some(1),
                    1024,
                    Arc::new(|| 0.0_f64),
                )
                .map_err(cache_helper_error)?;
                cache
                    .set("small", CacheValue::Text("ok".to_owned()), None)
                    .map_err(cache_helper_error)?;
                cache
                    .set("large", CacheValue::Text("x".repeat(2000)), None)
                    .map_err(cache_helper_error)?;
                Ok(json!({
                    "small": cache.get("small").map_err(cache_helper_error)?,
                    "large": cache.get("large").map_err(cache_helper_error)?,
                }))
            }
            Input::CachedJsonProjection { value } => projection(value),
            Input::DualLocalDefaultsErrors => dual::run(),
        }
    }
}

fn defaults() -> crate::helpers::HelperResult {
    let cache = LocalTtlCache::with_clock(None, None, None, 1024, Arc::new(|| 0.0_f64))
        .map_err(cache_helper_error)?;
    let zero = LocalTtlCache::with_clock(None, Some(0.0_f64), Some(0), 1024, Arc::new(|| 0.0_f64))
        .map_err(cache_helper_error)?;
    Ok(
        json!({"capacity": cache.as_ref().capacity, "ttl": cache.as_ref().default_ttl, "item_kib": cache.as_ref().max_size_per_item, "zero_ttl": zero.as_ref().default_ttl, "zero_item_kib": zero.as_ref().max_size_per_item}),
    )
}

fn expiry() -> crate::helpers::HelperResult {
    let mut cache = LocalTtlCache::with_clock(
        Some(2),
        Some(600.0_f64),
        Some(1024),
        1024,
        Arc::new(|| 10.0_f64),
    )
    .map_err(cache_helper_error)?;
    cache
        .set("b", CacheValue::Integer(2), Some(5.0_f64))
        .map_err(cache_helper_error)?;
    cache
        .set("a", CacheValue::Integer(1), Some(5.0_f64))
        .map_err(cache_helper_error)?;
    cache
        .set("c", CacheValue::Integer(3), Some(5.0_f64))
        .map_err(cache_helper_error)?;
    Ok(json!({
        "a": cache.get("a").map_err(cache_helper_error)?,
        "b": cache.get("b").map_err(cache_helper_error)?,
        "c": cache.get("c").map_err(cache_helper_error)?,
    }))
}

fn nonrefreshing() -> crate::helpers::HelperResult {
    let clock = Arc::new(AtomicU64::new(0.0_f64.to_bits()));
    let source = Arc::clone(&clock);
    let mut cache = LocalTtlCache::with_clock(
        Some(2),
        None,
        None,
        1024,
        Arc::new(move || f64::from_bits(source.load(Ordering::Relaxed))),
    )
    .map_err(cache_helper_error)?;
    cache
        .set("key", CacheValue::Integer(1), Some(10.0_f64))
        .map_err(cache_helper_error)?;
    cache
        .set("key", CacheValue::Integer(2), Some(100.0_f64))
        .map_err(cache_helper_error)?;
    clock.store(10.0_f64.to_bits(), Ordering::Relaxed);
    let boundary = cache.get("key").map_err(cache_helper_error)?;
    clock.store(11.0_f64.to_bits(), Ordering::Relaxed);
    let expired = cache.get("key").map_err(cache_helper_error)?;
    cache
        .set("deleted", CacheValue::Integer(3), None)
        .map_err(cache_helper_error)?;
    cache.delete("deleted");
    Ok(json!({
        "boundary": boundary,
        "expired": expired,
        "deleted": cache.get("deleted").map_err(cache_helper_error)?,
    }))
}

fn projection(value: &str) -> crate::helpers::HelperResult {
    let mut cache = LocalTtlCache::with_clock(None, None, None, 1024, Arc::new(|| 0.0_f64))
        .map_err(cache_helper_error)?;
    cache
        .set("value", CacheValue::Text(value.to_owned()), None)
        .map_err(cache_helper_error)?;
    Ok(json!({"value": cache.get("value").map_err(cache_helper_error)?}))
}

fn cache_helper_error(error: CacheError) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}
