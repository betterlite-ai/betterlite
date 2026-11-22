use std::future::Future;

use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::dual_cache::DualCache;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_redis::redis_cache::error::RedisCacheError;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive as _, ToPrimitive as _};

use crate::spend_reseed::cache::ReseedCache;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::values::{ReseedCacheValue, cache_amount};

// proxy_server.py's in-memory marker the authoritative floor read is cached under
pub(super) const SPEND_DB_FLOOR_MARKER_PREFIX: &str = "spend_db_floor:";

// the writes the read path needs beyond B1736's ReseedCache: a monotonic raise and a ttl'd memory set
pub trait SpendReadCache: ReseedCache {
    fn redis_set_max(
        &self,
        counter_key: &str,
        value: Decimal,
    ) -> impl Future<Output = Result<Option<Decimal>, Self::Error>> + Send;

    fn memory_set_ttl(
        &self,
        counter_key: &str,
        value: Decimal,
        ttl_seconds: Option<f64>,
    ) -> Result<(), Self::Error>;
}

impl<Memory: MemoryCacheOps> SpendReadCache for DualCache<Memory> {
    // LiteLLM's async_set_max(key, value): the Lua compares and raises, then refreshes the default ttl
    async fn redis_set_max(
        &self,
        counter_key: &str,
        value: Decimal,
    ) -> Result<Option<Decimal>, ReseedCacheError> {
        let held = match self.redis_cache.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        let Some(redis) = held else {
            return Ok(None);
        };
        let Some(amount) = value.to_f64() else {
            return Err(ReseedCacheError::OutOfRange(value));
        };
        let repaired = redis
            .counters
            .async_set_max(counter_key, amount, None)
            .await
            .map_err(RedisCacheError::from)?;
        Ok(repaired.and_then(Decimal::from_f64))
    }

    fn memory_set_ttl(
        &self,
        counter_key: &str,
        value: Decimal,
        ttl_seconds: Option<f64>,
    ) -> Result<(), ReseedCacheError> {
        let Some(amount) = value.to_f64() else {
            return Err(ReseedCacheError::OutOfRange(value));
        };
        self.in_memory_cache
            .set_cache(counter_key, CacheValue::Float(amount), ttl_seconds)
            .map_err(ReseedCacheError::Memory)
    }
}

// Python's float(value) on a cache hit: a value no number conversion accepts reads as absent
pub(super) fn cache_estimate(
    value: &ReseedCacheValue,
    counter_key: &str,
    tier: &str,
) -> Option<Decimal> {
    match cache_amount(value) {
        Ok(amount) => Some(amount),
        Err(error) => {
            tracing::debug!(
                counter_key,
                tier,
                %error,
                "the spend counter value is not a number"
            );
            None
        }
    }
}
