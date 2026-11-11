use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use betterlite_redis::request_batch::scope::active_request_redis_batch;

use super::{DualCache, DualCacheError, lock_read, value};
use crate::backend::memory::MemoryCacheOps;

// InMemoryCache.get_cache answers None for a missing key and for a stored None; both read as a miss
pub(crate) fn memory_value<Memory>(
    cache: &DualCache<Memory>,
    key: &str,
) -> Result<Option<Json>, DualCacheError>
where
    Memory: MemoryCacheOps,
{
    let stored = match cache.in_memory_cache.get_cache(key) {
        Ok(value) => value,
        Err(source) => return Err(DualCacheError::Memory { source }),
    };
    match stored {
        None | Some(CacheValue::Null) => Ok(None),
        Some(value) => value::json_from_cache(&value).map(Some),
    }
}

pub(crate) fn local_get<Memory>(
    cache: &DualCache<Memory>,
    key: &str,
    message: &'static str,
) -> Option<Json>
where
    Memory: MemoryCacheOps,
{
    match memory_value(cache, key) {
        Ok(hit) => hit,
        Err(error) => {
            tracing::error!(%error, "{message}");
            None
        }
    }
}

pub(crate) fn get<Memory>(cache: &DualCache<Memory>, key: &str) -> Option<Json>
where
    Memory: MemoryCacheOps,
{
    match memory_value(cache, key) {
        Ok(Some(hit)) => return Some(hit),
        Ok(None) => {}
        Err(error) => {
            tracing::error!(%error, "LiteLLM Cache: exception in get_cache");
            return None;
        }
    }
    redis_get(cache, key)
}

pub(crate) async fn async_get<Memory>(cache: &DualCache<Memory>, key: &str) -> Option<Json>
where
    Memory: MemoryCacheOps,
{
    match memory_value(cache, key) {
        Ok(Some(hit)) => return Some(hit),
        Ok(None) => {}
        Err(error) => {
            tracing::error!(%error, "LiteLLM Cache: exception in async_get_cache");
            return None;
        }
    }
    async_redis_get(cache, key).await
}

pub(crate) fn store_backfill<Memory>(
    cache: &DualCache<Memory>,
    key: &str,
    value: &Json,
) -> Result<(), DualCacheError>
where
    Memory: MemoryCacheOps,
{
    let ttl = *lock_read(&cache.default_in_memory_ttl);
    let stored = value::cache_from_json(value)?;
    cache
        .in_memory_cache
        .set_cache(key, stored, ttl)
        .map_err(|source| DualCacheError::Memory { source })
}

pub(crate) fn redis_get<Memory>(cache: &DualCache<Memory>, key: &str) -> Option<Json>
where
    Memory: MemoryCacheOps,
{
    let redis = cache.redis()?;
    let found = match redis.values.get_cache(key) {
        Ok(cached) => cached,
        Err(error) => {
            tracing::error!(%error, "LiteLLM Cache: exception in get_cache");
            return None;
        }
    };
    let hit = found?;
    let json = value::json_from_cached(hit);
    match store_backfill(cache, key, &json) {
        Ok(()) => Some(json),
        Err(error) => {
            tracing::error!(%error, "LiteLLM Cache: exception in get_cache");
            None
        }
    }
}

pub(crate) async fn async_redis_get<Memory>(cache: &DualCache<Memory>, key: &str) -> Option<Json>
where
    Memory: MemoryCacheOps,
{
    let redis = cache.redis()?;
    if let Some(batch) = active_request_redis_batch(&redis)
        && batch.read_as_missing(key)
    {
        return None;
    }
    let found = match redis.async_get_cache(key).await {
        Ok(cached) => cached,
        Err(error) => {
            tracing::error!(%error, "LiteLLM Cache: exception in async_get_cache");
            return None;
        }
    };
    let hit = found?;
    let json = value::json_from_cached(hit);
    match store_backfill(cache, key, &json) {
        Ok(()) => Some(json),
        Err(error) => {
            tracing::error!(%error, "LiteLLM Cache: exception in async_get_cache");
            None
        }
    }
}
