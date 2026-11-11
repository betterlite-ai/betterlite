use std::sync::Arc;

use betterlite_redis::redis_cache::RedisCache;
use num_traits::ToPrimitive as _;

use super::{DualCache, DualCacheError, MAX_REDIS_BATCH_CACHE_SIZE, lock_write};
use crate::backend::memory::MemoryCacheOps;

pub(crate) fn attach_redis<Memory>(
    cache: &DualCache<Memory>,
    redis: Arc<RedisCache>,
    default_redis_ttl: Option<f64>,
) where
    Memory: MemoryCacheOps,
{
    let mut held = lock_write(&cache.redis_cache);
    if held.is_some() {
        return;
    }
    *held = Some(redis);
    drop(held);
    if let Some(ttl) = default_redis_ttl {
        *lock_write(&cache.default_redis_ttl) = Some(ttl);
    }
}

pub(crate) fn redis_flush<Memory>(cache: &DualCache<Memory>) -> Result<(), DualCacheError>
where
    Memory: MemoryCacheOps,
{
    let Some(redis) = cache.redis() else {
        return Ok(());
    };
    redis
        .admin
        .flush_cache()
        .map_err(|source| DualCacheError::RedisDelete { source })
}

pub(crate) fn redis_delete<Memory>(
    cache: &DualCache<Memory>,
    key: &str,
) -> Result<(), DualCacheError>
where
    Memory: MemoryCacheOps,
{
    let Some(redis) = cache.redis() else {
        return Ok(());
    };
    redis
        .admin
        .delete_cache(key)
        .map_err(|source| DualCacheError::RedisDelete { source })
}

pub(crate) async fn async_redis_delete<Memory>(
    cache: &DualCache<Memory>,
    key: &str,
) -> Result<(), DualCacheError>
where
    Memory: MemoryCacheOps,
{
    let Some(redis) = cache.redis() else {
        return Ok(());
    };
    let _deleted = redis
        .admin
        .async_delete_cache(key)
        .await
        .map_err(|source| DualCacheError::RedisDelete { source })?;
    Ok(())
}

// Redis takes the whole key list as one DEL, so a large batch is split into bounded commands
pub(crate) async fn redis_delete_keys<Memory>(
    cache: &DualCache<Memory>,
    keys: &[String],
) -> Result<(), DualCacheError>
where
    Memory: MemoryCacheOps,
{
    let Some(redis) = cache.redis() else {
        return Ok(());
    };
    for chunk in keys.chunks(MAX_REDIS_BATCH_CACHE_SIZE) {
        redis
            .admin
            .delete_cache_keys(chunk)
            .await
            .map_err(|source| DualCacheError::RedisDelete { source })?;
    }
    Ok(())
}

pub(crate) async fn redis_ttl<Memory>(
    cache: &DualCache<Memory>,
    key: &str,
) -> Result<Option<f64>, DualCacheError>
where
    Memory: MemoryCacheOps,
{
    let Some(redis) = cache.redis() else {
        return Ok(None);
    };
    let remaining = redis
        .async_get_ttl(key)
        .await
        .map_err(|source| DualCacheError::RedisTtl { source })?;
    remaining.map_or(Ok(None), |seconds| Ok(seconds.to_f64()))
}

pub(crate) async fn redis_set_add<Memory>(
    cache: &DualCache<Memory>,
    key: &str,
    values: &[String],
    ttl: Option<f64>,
) -> Result<(), DualCacheError>
where
    Memory: MemoryCacheOps,
{
    let Some(redis) = cache.redis() else {
        return Ok(());
    };
    redis
        .values
        .async_set_cache_sadd(key, values, ttl)
        .await
        .map_err(|source| DualCacheError::RedisSetAdd { source })
}
