use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::CachedValue;

use super::{DualCache, DualCacheError, lock_read, logged, value};
use crate::backend::memory::MemoryCacheOps;

// InMemoryCache answers None for a missing key and for a stored None; both read as a miss
pub(super) fn memory_hit<Memory>(
    cache: &DualCache<Memory>,
    key: &str,
) -> Result<Option<Json>, DualCacheError>
where
    Memory: MemoryCacheOps,
{
    match cache.in_memory_cache.get_cache(key)? {
        None | Some(CacheValue::Null) => Ok(None),
        Some(stored) => value::convert(&stored).map(Some),
    }
}

// LiteLLM's _backfill_kwargs: a Redis hit lands in memory with the configured memory ttl, a null is a miss
pub(super) fn backfill<Memory>(
    cache: &DualCache<Memory>,
    key: &str,
    hit: CachedValue,
) -> Option<Json>
where
    Memory: MemoryCacheOps,
{
    if hit == CachedValue::Null {
        return None;
    }
    let json = value::json_from_cached(hit);
    let ttl = *lock_read(&cache.default_in_memory_ttl);
    let written = value::cache_from_json(&json)
        .and_then(|stored| Ok(cache.in_memory_cache.set_cache(key, stored, ttl)?));
    logged(written, "LiteLLM Cache: exception in get_cache");
    Some(json)
}
