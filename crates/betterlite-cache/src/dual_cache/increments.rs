use std::sync::Arc;

use betterlite_redis::batch::RedisBatch;
use betterlite_redis::batch::error::BatchError;
use betterlite_redis::batch::result::{Settled, SettledFollowUp};
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use num_traits::ToPrimitive as _;

use super::{DualCache, DualCacheError};
use crate::backend::{CacheResult, memory::MemoryCacheOps};
use crate::types::pipeline::RedisPipelineIncrement;

pub(crate) fn whole_increment(value: f64) -> Result<i64, DualCacheError> {
    let whole = value
        .to_i64()
        .ok_or(DualCacheError::FractionalIncrement { value })?;
    (whole.to_f64() == Some(value))
        .then_some(whole)
        .ok_or(DualCacheError::FractionalIncrement { value })
}

pub(crate) fn whole_seconds(ttl: Option<f64>) -> Result<Option<i64>, DualCacheError> {
    ttl.map_or_else(
        || Ok(None),
        |seconds| {
            seconds
                .trunc()
                .to_i64()
                .map(Some)
                .ok_or(DualCacheError::TtlOutOfRange { seconds })
        },
    )
}

pub(crate) fn memory_increment<Memory>(
    cache: &DualCache<Memory>,
    key: &str,
    value: f64,
    ttl: Option<f64>,
) -> Result<f64, DualCacheError>
where
    Memory: MemoryCacheOps,
{
    cache
        .in_memory_cache
        .increment_cache(key, value, ttl)
        .map_err(|source| DualCacheError::Memory { source })
}

// the memory pipeline forwards no ttl: LiteLLM's InMemoryCache.async_increment_pipeline ignores it
pub(crate) fn memory_increment_pipeline<Memory>(
    cache: &DualCache<Memory>,
    increments: &[RedisPipelineIncrement],
) -> CacheResult<Vec<f64>>
where
    Memory: MemoryCacheOps,
{
    let mut results = Vec::with_capacity(increments.len());
    for increment in increments {
        results.push(cache.in_memory_cache.increment_cache(
            &increment.key,
            increment.increment_value,
            None,
        )?);
    }
    Ok(results)
}

pub(crate) fn redis_increments(increments: &[RedisPipelineIncrement]) -> Vec<IncrementOperation> {
    increments
        .iter()
        .map(|increment| IncrementOperation {
            key: increment.key.clone(),
            increment_value: increment.increment_value,
            ttl: increment.ttl,
        })
        .collect()
}

pub(crate) fn redis_increment<Memory>(
    cache: &DualCache<Memory>,
    key: &str,
    value: f64,
    ttl: Option<f64>,
    memory: f64,
) -> Result<f64, DualCacheError>
where
    Memory: MemoryCacheOps,
{
    let Some(redis) = cache.redis() else {
        return Ok(memory);
    };
    let whole = whole_increment(value)?;
    let counted = redis
        .counters
        .increment_cache(key, whole, ttl)
        .map_err(|source| DualCacheError::RedisIncrement { source })?;
    counted
        .to_f64()
        .ok_or(DualCacheError::CountOutOfRange { value: counted })
}

pub(crate) struct RedisIncrement<'key> {
    pub(crate) key: &'key str,
    pub(crate) value: f64,
    pub(crate) ttl: Option<f64>,
    pub(crate) memory: Option<f64>,
    pub(crate) refresh_ttl: bool,
}

pub(crate) async fn async_redis_increment<Memory>(
    cache: &DualCache<Memory>,
    increment: RedisIncrement<'_>,
) -> Option<f64>
where
    Memory: MemoryCacheOps,
{
    let Some(redis) = cache.redis() else {
        return increment.memory;
    };
    let redis_ttl = match whole_seconds(increment.ttl) {
        Ok(seconds) => seconds,
        Err(error) => {
            tracing::warn!(%error, "Redis async_increment_cache failed, falling back to in-memory result");
            return increment.memory;
        }
    };
    match redis
        .async_increment(
            increment.key,
            increment.value,
            redis_ttl,
            increment.refresh_ttl,
        )
        .await
    {
        Ok(incremented) => Some(incremented),
        Err(error) => {
            tracing::warn!(%error, "Redis async_increment_cache failed, falling back to in-memory result");
            increment.memory
        }
    }
}

pub(crate) async fn async_redis_increment_pipeline<Memory>(
    cache: &DualCache<Memory>,
    increments: &[RedisPipelineIncrement],
    memory: Option<Vec<f64>>,
) -> Option<Vec<f64>>
where
    Memory: MemoryCacheOps,
{
    let Some(redis) = cache.redis() else {
        return memory;
    };
    let operations = redis_increments(increments);
    match redis.async_increment_pipeline(&operations).await {
        Ok(results) => results,
        Err(error) => {
            tracing::warn!(%error, "Redis async_increment_cache_pipeline failed, falling back to in-memory result");
            memory
        }
    }
}

pub(crate) fn log_deferred_increment_failure(settled: &Settled<f64>) -> Option<SettledFollowUp> {
    match settled {
        Ok(_incremented) => None,
        Err(error) if matches!(error.as_ref(), BatchError::Cancelled) => None,
        Err(error) => {
            tracing::warn!(%error, "post-call Redis increment failed");
            None
        }
    }
}

pub(crate) fn queue_post_call_increments<Memory>(
    cache: &DualCache<Memory>,
    batch: &Arc<RedisBatch>,
    increments: &[RedisPipelineIncrement],
) where
    Memory: MemoryCacheOps,
{
    if let Err(error) = memory_increment_pipeline(cache, increments) {
        tracing::warn!(%error, "in-memory increment failed");
    }
    for increment in increments {
        let settled = batch.increment(&increment.key, increment.increment_value, increment.ttl);
        settled.on_settled(Arc::new(log_deferred_increment_failure));
    }
}
