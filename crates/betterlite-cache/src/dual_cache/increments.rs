use std::sync::Arc;

use betterlite_redis::batch::RedisBatch;
use betterlite_redis::batch::error::BatchError;
use betterlite_redis::batch::result::{Settled, SettledFollowUp};
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use betterlite_redis::redis_cache::RedisCache;
use num_traits::ToPrimitive as _;

use super::{CacheOptions, DualCache, DualCacheError, logged};
use crate::backend::CacheResult;
use crate::backend::memory::MemoryCacheOps;
use crate::types::pipeline::RedisPipelineIncrement;

// LiteLLM's _log_deferred_increment_failure: a cancelled increment is not a failure worth a log line
pub(super) fn log_deferred_increment_failure(settled: &Settled<f64>) -> Option<SettledFollowUp> {
    if let Err(error) = settled
        && !matches!(error.as_ref(), BatchError::Cancelled)
    {
        tracing::warn!(%error, "post-call Redis increment failed");
    }
    None
}

// the memory pipeline forwards no ttl: LiteLLM's InMemoryCache.async_increment_pipeline ignores it
pub(super) fn memory_increments<Memory>(
    cache: &DualCache<Memory>,
    operations: &[RedisPipelineIncrement],
) -> CacheResult<Vec<f64>>
where
    Memory: MemoryCacheOps,
{
    operations
        .iter()
        .map(|operation| {
            let (key, amount) = (&operation.key, operation.increment_value);
            cache.in_memory_cache.increment_cache(key, amount, None)
        })
        .collect()
}

// Redis INCRBY takes whole numbers only, so a fractional amount fails after the memory tier counted it
pub(super) fn redis_increment(
    redis: &RedisCache,
    key: &str,
    value: f64,
    ttl: Option<f64>,
) -> Result<f64, DualCacheError> {
    let whole = value
        .to_i64()
        .filter(|whole| whole.to_f64() == Some(value))
        .ok_or(DualCacheError::FractionalIncrement { value })?;
    let counted = redis.counters.increment_cache(key, whole, ttl)?;
    counted
        .to_f64()
        .ok_or(DualCacheError::CountOutOfRange { value: counted })
}

// a Redis failure falls back to the count the memory tier already holds
pub(super) async fn async_redis_increment(
    redis: &RedisCache,
    key: &str,
    value: f64,
    options: CacheOptions,
    memory: f64,
) -> f64 {
    let ttl = options.ttl.and_then(|seconds| seconds.trunc().to_i64());
    let incremented = redis
        .async_increment(key, value, ttl, options.refresh_ttl)
        .await;
    let message = "Redis async_increment_cache failed, falling back to in-memory result";
    logged(incremented, message).unwrap_or(memory)
}

pub(super) async fn async_redis_pipeline(
    redis: &RedisCache,
    operations: &[RedisPipelineIncrement],
    memory: Vec<f64>,
) -> Option<Vec<f64>> {
    let queued: Vec<IncrementOperation> = operations
        .iter()
        .map(|operation| IncrementOperation {
            key: operation.key.clone(),
            increment_value: operation.increment_value,
            ttl: operation.ttl,
        })
        .collect();
    let message = "Redis async_increment_cache_pipeline failed, falling back to in-memory result";
    logged(redis.async_increment_pipeline(&queued).await, message).unwrap_or(Some(memory))
}

pub(super) fn queue_post_call(batch: &Arc<RedisBatch>, operations: &[RedisPipelineIncrement]) {
    for operation in operations {
        let queued = batch.increment(&operation.key, operation.increment_value, operation.ttl);
        queued.on_settled(Arc::new(log_deferred_increment_failure));
    }
}
