use std::sync::Arc;

use async_trait::async_trait;
use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::dual_cache::DualCache;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_redis::batch::RedisBatch;
use betterlite_redis::batch::result::SettledHook;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::request_batch::scope::active_post_call_redis_batch;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

use crate::spend_counter_batch::batch::PendingSpendIncrement;
use crate::spend_reseed::values::ReseedCacheValue;

use super::super::error::LayersError;
use super::{CounterLayers, CounterWriteBatch, PostCallBatch};

#[async_trait]
impl<Memory: MemoryCacheOps> CounterLayers for DualCache<Memory> {
    async fn increment_redis(
        &self,
        counter_key: &str,
        increment: Decimal,
    ) -> Result<Decimal, LayersError> {
        let redis = redis_tier(self).ok_or(LayersError::Unavailable)?;
        // LiteLLM's async_increment without an explicit ttl: the default TTL, refreshed
        let counted = redis
            .async_increment(counter_key, increment.as_f64(), None, true)
            .await?;
        Decimal::from_f64(counted).ok_or(LayersError::NonFinite(counted))
    }

    async fn refresh_redis_ttl(&self, counter_key: &str) -> Result<bool, LayersError> {
        let redis = redis_tier(self).ok_or(LayersError::Unavailable)?;
        Ok(redis.async_refresh_ttl(counter_key, None).await?)
    }

    async fn delete_redis(&self, counter_key: &str) -> Result<(), LayersError> {
        let redis = redis_tier(self).ok_or(LayersError::Unavailable)?;
        let _deleted = redis.admin.async_delete_cache(counter_key).await?;
        Ok(())
    }

    async fn delete_memory(&self, counter_key: &str) -> Result<(), LayersError> {
        self.in_memory_cache
            .delete_cache(counter_key)
            .map_err(LayersError::Memory)
    }

    async fn set_memory_value(
        &self,
        counter_key: &str,
        value: ReseedCacheValue,
    ) -> Result<(), LayersError> {
        let Some(stored) = memory_value(value) else {
            return Ok(());
        };
        self.in_memory_cache
            .set_cache(counter_key, stored, None)
            .map_err(LayersError::Memory)
    }

    async fn increment_redis_pipeline(
        &self,
        pending: &[PendingSpendIncrement],
    ) -> Result<Vec<Decimal>, LayersError> {
        let redis = redis_tier(self).ok_or(LayersError::Unavailable)?;
        let ttl = Some(redis.default_ttl);
        let mut operations = Vec::with_capacity(pending.len());
        for item in pending {
            operations.push(IncrementOperation {
                key: item.counter_key.clone(),
                increment_value: item.increment.as_f64(),
                ttl,
            });
        }
        let results = redis
            .async_increment_pipeline(&operations)
            .await?
            .unwrap_or_default();
        let mut amounts = Vec::with_capacity(results.len());
        for value in results {
            amounts.push(Decimal::from_f64(value).ok_or(LayersError::NonFinite(value))?);
        }
        Ok(amounts)
    }

    fn post_call(&self) -> Option<PostCallBatch> {
        let redis = redis_tier(self)?;
        let batch = active_post_call_redis_batch(&redis)?;
        Some(PostCallBatch {
            batch: Arc::new(RedisWriteBatch { batch }),
            ttl: redis.default_ttl,
        })
    }
}

struct RedisWriteBatch {
    batch: Arc<RedisBatch>,
}

impl CounterWriteBatch for RedisWriteBatch {
    fn increment(&self, counter_key: &str, increment: f64, ttl: i64, hook: SettledHook<f64>) {
        let queued = self.batch.increment(counter_key, increment, Some(ttl));
        queued.on_settled(hook);
    }
}

// DualCache keeps its Redis tier behind a crate-private accessor, so this reaches it the way spend_reseed does
fn redis_tier<Memory>(cache: &DualCache<Memory>) -> Option<Arc<RedisCache>>
where
    Memory: MemoryCacheOps,
{
    match cache.redis_cache.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

// an undecodable stored value has no in-memory shape of its own, so warm detection leaves it to Redis
fn memory_value(value: ReseedCacheValue) -> Option<CacheValue> {
    match value {
        ReseedCacheValue::Number(amount) => Some(CacheValue::Float(amount.as_f64())),
        ReseedCacheValue::Text(text) => Some(CacheValue::Text(text)),
        ReseedCacheValue::Boolean(flag) => Some(CacheValue::Boolean(flag)),
        ReseedCacheValue::Unreadable => None,
    }
}
