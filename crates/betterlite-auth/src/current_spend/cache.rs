use std::future::Future;

use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::dual_cache::DualCache;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_redis::redis_cache::error::RedisCacheError;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive as _, ToPrimitive as _};

use crate::spend_counter_batch::batch::{CounterValue, SpendCounterBatchError};
use crate::spend_counter_batch::scope::SpendCounterScope;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::cache::{ActiveSpendBatch, ReseedCache};

// the spend counter cache's two writes the reseed reads never make: an expiring floor marker and Redis SET MAX
pub trait CounterCache: ReseedCache {
    fn memory_set_expiring(
        &self,
        key: &str,
        value: Decimal,
        ttl_seconds: u32,
    ) -> Result<(), Self::Error>;

    fn redis_set_max(
        &self,
        counter_key: &str,
        value: Decimal,
    ) -> impl Future<Output = Result<Option<Decimal>, Self::Error>> + Send;
}

pub trait CounterBatch: ActiveSpendBatch {
    fn forget(&self, counter_key: &str);
}

impl<Memory: MemoryCacheOps> CounterCache for DualCache<Memory> {
    fn memory_set_expiring(
        &self,
        key: &str,
        value: Decimal,
        ttl_seconds: u32,
    ) -> Result<(), ReseedCacheError> {
        let amount = value.to_f64().ok_or(ReseedCacheError::OutOfRange(value))?;
        let ttl = Some(f64::from(ttl_seconds));
        self.in_memory_cache
            .set_cache(key, CacheValue::Float(amount), ttl)
            .map_err(ReseedCacheError::Memory)
    }

    async fn redis_set_max(
        &self,
        counter_key: &str,
        value: Decimal,
    ) -> Result<Option<Decimal>, ReseedCacheError> {
        let configured = match self.redis_cache.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        let Some(redis) = configured else {
            return Ok(None);
        };
        let amount = value.to_f64().ok_or(ReseedCacheError::OutOfRange(value))?;
        let stored = redis
            .counters
            .async_set_max(counter_key, amount, None)
            .await
            .map_err(|source| ReseedCacheError::Redis(RedisCacheError::from(source)))?;
        // a non-finite reply names no counter value, so the batch entry stays forgotten
        Ok(stored.and_then(Decimal::from_f64))
    }
}

impl ActiveSpendBatch for SpendCounterScope {
    async fn read(
        &self,
        counter_key: &str,
    ) -> Result<Option<CounterValue>, SpendCounterBatchError> {
        self.read_spend_counter(counter_key).await
    }

    fn record(&self, counter_key: &str, value: Decimal) {
        self.record_spend_counter_value(counter_key, value);
    }
}

impl CounterBatch for SpendCounterScope {
    fn forget(&self, counter_key: &str) {
        self.forget_spend_counter(counter_key);
    }
}
