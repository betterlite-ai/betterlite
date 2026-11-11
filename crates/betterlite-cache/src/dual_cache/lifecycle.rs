use std::sync::Arc;

use betterlite_redis::redis_cache::RedisCache;
use num_traits::ToPrimitive as _;

use super::{DualCacheError, MAX_REDIS_BATCH_CACHE_SIZE};

pub(crate) fn redis_flush(redis: Option<&Arc<RedisCache>>) -> Result<(), DualCacheError> {
    redis.map_or_else(
        || Ok(()),
        |tier| tier.admin.flush_cache().map_err(DualCacheError::from),
    )
}

pub(crate) fn redis_delete(
    redis: Option<&Arc<RedisCache>>,
    key: &str,
) -> Result<(), DualCacheError> {
    redis.map_or_else(
        || Ok(()),
        |tier| tier.admin.delete_cache(key).map_err(DualCacheError::from),
    )
}

pub(crate) async fn async_redis_delete(
    redis: Option<&Arc<RedisCache>>,
    key: &str,
) -> Result<(), DualCacheError> {
    let Some(tier) = redis else {
        return Ok(());
    };
    let _deleted = tier
        .admin
        .async_delete_cache(key)
        .await
        .map_err(DualCacheError::from)?;
    Ok(())
}

// Redis takes the whole key list as one DEL, so a large batch is split into bounded commands
pub(crate) async fn redis_delete_keys(
    redis: Option<&Arc<RedisCache>>,
    keys: &[String],
) -> Result<(), DualCacheError> {
    let Some(tier) = redis else {
        return Ok(());
    };
    for chunk in keys.chunks(MAX_REDIS_BATCH_CACHE_SIZE) {
        tier.admin
            .delete_cache_keys(chunk)
            .await
            .map_err(DualCacheError::from)?;
    }
    Ok(())
}

pub(crate) async fn redis_ttl(
    redis: Option<&Arc<RedisCache>>,
    key: &str,
) -> Result<Option<f64>, DualCacheError> {
    let Some(tier) = redis else {
        return Ok(None);
    };
    let remaining = tier
        .async_get_ttl(key)
        .await
        .map_err(DualCacheError::from)?;
    remaining.map_or(Ok(None), |seconds| Ok(seconds.to_f64()))
}

pub(crate) async fn redis_set_add(
    redis: Option<&Arc<RedisCache>>,
    key: &str,
    values: &[String],
    ttl: Option<f64>,
) -> Result<(), DualCacheError> {
    let Some(tier) = redis else {
        return Ok(());
    };
    tier.values
        .async_set_cache_sadd(key, values, ttl)
        .await
        .map_err(DualCacheError::from)
}
