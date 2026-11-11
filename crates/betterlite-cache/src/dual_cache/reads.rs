use std::sync::Arc;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::request_batch::scope::active_request_redis_batch;

use super::{logged, value};
use crate::backend::memory::MemoryCacheOps;

// InMemoryCache answers None for a missing key and for a stored None; both read as a miss
pub(crate) fn memory_hit(memory: &dyn MemoryCacheOps, key: &str) -> Option<Json> {
    let stored = logged(
        memory.get_cache(key),
        "LiteLLM Cache: exception in get_cache",
    )??;
    if matches!(stored, CacheValue::Null) {
        return None;
    }
    value::convert(&stored).ok()
}

fn backfill(
    memory: &dyn MemoryCacheOps,
    ttl: Option<f64>,
    key: &str,
    hit: CachedValue,
) -> Option<Json> {
    let json = value::json_from_cached(hit);
    let stored = value::cache_from_json(&json).ok()?;
    logged(
        memory.set_cache(key, stored, ttl),
        "LiteLLM Cache: exception in get_cache",
    )?;
    Some(json)
}

pub(crate) fn redis_read(
    redis: &Arc<RedisCache>,
    memory: &dyn MemoryCacheOps,
    ttl: Option<f64>,
    key: &str,
) -> Option<Json> {
    let found = redis.values.get_cache(key).unwrap_or_else(|error| {
        tracing::error!(%error, "LiteLLM Cache: exception in get_cache");
        None
    });
    backfill(memory, ttl, key, found?)
}

pub(crate) async fn async_redis_read(
    redis: &Arc<RedisCache>,
    memory: &dyn MemoryCacheOps,
    ttl: Option<f64>,
    key: &str,
) -> Option<Json> {
    if active_request_redis_batch(redis).is_some_and(|batch| batch.read_as_missing(key)) {
        return None;
    }
    let found = redis.async_get_cache(key).await.unwrap_or_else(|error| {
        tracing::error!(%error, "LiteLLM Cache: exception in async_get_cache");
        None
    });
    backfill(memory, ttl, key, found?)
}
