use std::sync::Arc;

use betterlite_cache::backend::BackendError;
use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::dual_cache::{CacheOptions, DualCache};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::error::RedisCacheError;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive as _, ToPrimitive as _};
use thiserror::Error;

use crate::model_budget_values::amount_text::amount_from_text;

use super::super::values::ReseedCacheValue;
use super::ReseedCache;

#[derive(Debug, Error)]
pub enum ReseedCacheError {
    #[error(transparent)]
    Redis(#[from] RedisCacheError),
    #[error(transparent)]
    Values(#[from] RedisCacheValuesError),
    #[error(transparent)]
    Memory(#[from] BackendError),
    #[error("the spend {0} does not fit a float cache value")]
    OutOfRange(Decimal),
}

impl<Memory: MemoryCacheOps> ReseedCache for DualCache<Memory> {
    type Error = ReseedCacheError;

    fn redis_configured(&self) -> bool {
        redis_of(self).is_some()
    }

    async fn redis_get(
        &self,
        counter_key: &str,
    ) -> Result<Option<ReseedCacheValue>, ReseedCacheError> {
        let Some(redis) = redis_of(self) else {
            return Ok(None);
        };
        let read = redis.async_get_cache(counter_key).await?;
        Ok(read.map(StoredValue::Redis).and_then(stored_value))
    }

    async fn redis_set_nx(
        &self,
        counter_key: &str,
        value: Decimal,
    ) -> Result<Option<bool>, ReseedCacheError> {
        let Some(redis) = redis_of(self) else {
            return Ok(None);
        };
        let amount = value.to_f64().ok_or(ReseedCacheError::OutOfRange(value))?;
        Ok(redis
            .values
            .async_set_cache(Some(counter_key), &amount, None, true)
            .await?)
    }

    async fn memory_get(
        &self,
        counter_key: &str,
    ) -> Result<Option<ReseedCacheValue>, ReseedCacheError> {
        let stored = self.in_memory_cache.get_cache(counter_key)?;
        Ok(stored.map(StoredValue::Memory).and_then(stored_value))
    }

    async fn memory_set(&self, counter_key: &str, value: Decimal) -> Result<(), ReseedCacheError> {
        let amount = value.to_f64().ok_or(ReseedCacheError::OutOfRange(value))?;
        Ok(self
            .in_memory_cache
            .set_cache(counter_key, CacheValue::Float(amount), None)?)
    }

    async fn increment_local(
        &self,
        counter_key: &str,
        increment: Decimal,
    ) -> Result<Option<Decimal>, ReseedCacheError> {
        let amount = increment
            .to_f64()
            .ok_or(ReseedCacheError::OutOfRange(increment))?;
        let options = CacheOptions {
            ttl: None,
            local_only: true,
            refresh_ttl: true,
        };
        let counted = self
            .async_increment_cache(counter_key, amount, options)
            .await;
        Ok(counted.and_then(Decimal::from_f64))
    }
}

fn redis_of<Memory>(cache: &DualCache<Memory>) -> Option<Arc<RedisCache>>
where
    Memory: MemoryCacheOps,
{
    match cache.redis_cache.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

pub(crate) enum StoredValue {
    Redis(CachedValue),
    Memory(CacheValue),
}

// the two cache tiers store the same shapes; a Redis number keeps its exact text, a memory number is a float
pub(crate) fn stored_value(value: StoredValue) -> Option<ReseedCacheValue> {
    match value {
        StoredValue::Redis(CachedValue::Null) | StoredValue::Memory(CacheValue::Null) => None,
        StoredValue::Redis(CachedValue::Boolean(flag))
        | StoredValue::Memory(CacheValue::Boolean(flag)) => Some(ReseedCacheValue::Boolean(flag)),
        StoredValue::Redis(CachedValue::Text(text))
        | StoredValue::Memory(CacheValue::Text(text)) => Some(ReseedCacheValue::Text(text)),
        StoredValue::Redis(CachedValue::Number(number)) => Some(
            amount_from_text(&number.to_string())
                .map_or(ReseedCacheValue::Unreadable, ReseedCacheValue::Number),
        ),
        StoredValue::Memory(CacheValue::Integer(count)) => {
            Some(ReseedCacheValue::Number(Decimal::from(count)))
        }
        StoredValue::Memory(CacheValue::Float(amount)) => Some(
            Decimal::from_f64(amount)
                .map_or(ReseedCacheValue::Unreadable, ReseedCacheValue::Number),
        ),
        _ => Some(ReseedCacheValue::Unreadable),
    }
}
