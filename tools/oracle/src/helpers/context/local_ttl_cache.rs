use crate::helpers::OrHelperError as _;
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

type CacheResult = Result<serde_json::Value, CacheError>;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let value = match input {
            Input::CacheDefaults => defaults(),
            Input::ExpiryAndOverwrite => expiry(),
            Input::NonrefreshingTtl => nonrefreshing(),
            Input::ValueSizePolicy => value_size_policy(),
            Input::CachedJsonProjection { value } => projection(value),
            Input::DualLocalDefaultsErrors => return dual::run(),
        };
        value.or_value_error()
    }
}

fn value_size_policy() -> CacheResult {
    let mut cache = LocalTtlCache::with_clock(
        Some(2),
        Some(600.0_f64),
        Some(1),
        1024,
        Arc::new(|| 0.0_f64),
    )?;
    cache.set("small", CacheValue::Text("ok".to_owned()), None)?;
    cache.set("large", CacheValue::Text("x".repeat(2000)), None)?;
    Ok(json!({"small": cache.get("small")?, "large": cache.get("large")?}))
}

fn defaults() -> CacheResult {
    let cache = LocalTtlCache::with_clock(None, None, None, 1024, Arc::new(|| 0.0_f64))?;
    let zero = LocalTtlCache::with_clock(None, Some(0.0_f64), Some(0), 1024, Arc::new(|| 0.0_f64))?;
    Ok(
        json!({"capacity": cache.as_ref().capacity, "ttl": cache.as_ref().default_ttl, "item_kib": cache.as_ref().max_size_per_item, "zero_ttl": zero.as_ref().default_ttl, "zero_item_kib": zero.as_ref().max_size_per_item}),
    )
}

fn expiry() -> CacheResult {
    let mut cache = LocalTtlCache::with_clock(
        Some(2),
        Some(600.0_f64),
        Some(1024),
        1024,
        Arc::new(|| 10.0_f64),
    )?;
    cache.set("b", CacheValue::Integer(2), Some(5.0_f64))?;
    cache.set("a", CacheValue::Integer(1), Some(5.0_f64))?;
    cache.set("c", CacheValue::Integer(3), Some(5.0_f64))?;
    Ok(json!({"a": cache.get("a")?, "b": cache.get("b")?, "c": cache.get("c")?}))
}

fn nonrefreshing() -> CacheResult {
    let clock = Arc::new(AtomicU64::new(0.0_f64.to_bits()));
    let source = Arc::clone(&clock);
    let mut cache = LocalTtlCache::with_clock(
        Some(2),
        None,
        None,
        1024,
        Arc::new(move || f64::from_bits(source.load(Ordering::Relaxed))),
    )?;
    cache.set("key", CacheValue::Integer(1), Some(10.0_f64))?;
    cache.set("key", CacheValue::Integer(2), Some(100.0_f64))?;
    clock.store(10.0_f64.to_bits(), Ordering::Relaxed);
    let boundary = cache.get("key")?;
    clock.store(11.0_f64.to_bits(), Ordering::Relaxed);
    let expired = cache.get("key")?;
    cache.set("deleted", CacheValue::Integer(3), None)?;
    cache.delete("deleted");
    Ok(json!({"boundary": boundary, "expired": expired, "deleted": cache.get("deleted")?}))
}

fn projection(value: &str) -> CacheResult {
    let mut cache = LocalTtlCache::with_clock(None, None, None, 1024, Arc::new(|| 0.0_f64))?;
    cache.set("value", CacheValue::Text(value.to_owned()), None)?;
    Ok(json!({"value": cache.get("value")?}))
}
