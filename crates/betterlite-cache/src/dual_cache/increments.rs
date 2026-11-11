use std::sync::Arc;

use betterlite_redis::batch::RedisBatch;
use betterlite_redis::batch::error::BatchError;
use betterlite_redis::batch::result::{Settled, SettledFollowUp};
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use betterlite_redis::redis_cache::RedisCache;
use num_traits::ToPrimitive as _;

use super::DualCacheError;
use crate::backend::{CacheResult, memory::MemoryCacheOps};
use crate::types::pipeline::RedisPipelineIncrement;

pub(crate) fn memory_increment(
    memory: &dyn MemoryCacheOps,
    key: &str,
    value: f64,
    ttl: Option<f64>,
) -> Result<f64, DualCacheError> {
    memory
        .increment_cache(key, value, ttl)
        .map_err(|source| DualCacheError::Memory { source })
}

// the memory pipeline forwards no ttl: LiteLLM's InMemoryCache.async_increment_pipeline ignores it
pub(crate) fn memory_increment_pipeline(
    memory: &dyn MemoryCacheOps,
    operations: &[RedisPipelineIncrement],
) -> CacheResult<Vec<f64>> {
    operations
        .iter()
        .map(|operation| memory.increment_cache(&operation.key, operation.increment_value, None))
        .collect()
}

pub(crate) fn redis_increments(operations: &[RedisPipelineIncrement]) -> Vec<IncrementOperation> {
    operations
        .iter()
        .map(|operation| IncrementOperation {
            key: operation.key.clone(),
            increment_value: operation.increment_value,
            ttl: operation.ttl,
        })
        .collect()
}

pub(crate) fn redis_increment(
    redis: Option<&Arc<RedisCache>>,
    key: &str,
    value: f64,
    ttl: Option<f64>,
    memory: f64,
) -> Result<f64, DualCacheError> {
    let Some(tier) = redis else {
        return Ok(memory);
    };
    let whole = value
        .to_i64()
        .filter(|whole| whole.to_f64() == Some(value))
        .ok_or(DualCacheError::FractionalIncrement { value })?;
    let counted = tier
        .counters
        .increment_cache(key, whole, ttl)
        .map_err(DualCacheError::from)?;
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

pub(crate) async fn async_redis_increment(
    redis: Option<&Arc<RedisCache>>,
    increment: RedisIncrement<'_>,
) -> Option<f64> {
    let Some(tier) = redis else {
        return increment.memory;
    };
    let ttl = match increment.ttl {
        None => None,
        Some(seconds) => {
            let Some(whole) = seconds.trunc().to_i64() else {
                tracing::warn!("the ttl is outside the whole seconds Redis can hold");
                return increment.memory;
            };
            Some(whole)
        }
    };
    match tier
        .async_increment(increment.key, increment.value, ttl, increment.refresh_ttl)
        .await
    {
        Ok(incremented) => Some(incremented),
        Err(error) => {
            tracing::warn!(%error, "Redis async_increment_cache failed, falling back to in-memory result");
            increment.memory
        }
    }
}

pub(crate) async fn async_redis_increment_pipeline(
    redis: Option<&Arc<RedisCache>>,
    operations: &[RedisPipelineIncrement],
    memory: Option<Vec<f64>>,
) -> Option<Vec<f64>> {
    let Some(tier) = redis else {
        return memory;
    };
    let increment_operations = redis_increments(operations);
    match tier.async_increment_pipeline(&increment_operations).await {
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

pub(crate) fn queue_post_call_increments(
    memory: &dyn MemoryCacheOps,
    batch: &Arc<RedisBatch>,
    operations: &[RedisPipelineIncrement],
) {
    if let Err(error) = memory_increment_pipeline(memory, operations) {
        tracing::warn!(%error, "in-memory increment failed");
    }
    for operation in operations {
        batch
            .increment(&operation.key, operation.increment_value, operation.ttl)
            .on_settled(Arc::new(log_deferred_increment_failure));
    }
}
