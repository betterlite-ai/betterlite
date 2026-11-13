use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use betterlite_redis::redis_cache::RedisCache;
use rust_decimal::Decimal;

use super::error::IncrementError;
use super::queue::QueuedIncrement;
use super::window::WindowRedis;

pub struct RedisCacheWindow {
    pub cache: Arc<RedisCache>,
}

#[async_trait]
impl WindowRedis for RedisCacheWindow {
    async fn read_start_time(&self, key: &str) -> Result<Option<f64>, IncrementError> {
        match self.cache.async_get_cache(key).await {
            Ok(Some(CachedValue::Number(number))) => {
                Ok(Some(number.as_f64().ok_or_else(|| {
                    IncrementError::InvalidCacheValue {
                        key: key.to_owned(),
                    }
                })?))
            }
            Ok(Some(_)) => Err(IncrementError::InvalidCacheValue {
                key: key.to_owned(),
            }),
            Ok(None) | Err(_) => Ok(None),
        }
    }

    async fn write_start_time(&self, key: &str, start_time: f64, ttl: Duration) {
        let _stored = self
            .cache
            .values
            .async_set_cache(Some(key), &start_time, Some(ttl.as_secs_f64()), false)
            .await;
    }

    async fn write_spend(&self, key: &str, cost: Decimal, ttl: Duration) {
        let _stored = self
            .cache
            .values
            .async_set_cache(Some(key), &cost.as_f64(), Some(ttl.as_secs_f64()), false)
            .await;
    }

    async fn write_increments(&self, operations: &[QueuedIncrement]) -> bool {
        let Ok(increments) = pipeline_operations(operations) else {
            return false;
        };
        self.cache
            .async_increment_pipeline(&increments)
            .await
            .is_ok()
    }
}

pub(crate) fn pipeline_operations(
    operations: &[QueuedIncrement],
) -> Result<Vec<IncrementOperation>, IncrementError> {
    let mut increments = Vec::with_capacity(operations.len());
    for operation in operations {
        let ttl_seconds = operation.ttl.as_secs();
        let ttl = i64::try_from(ttl_seconds).map_err(|_source| IncrementError::TtlOutOfRange {
            seconds: ttl_seconds,
        })?;
        increments.push(IncrementOperation {
            key: operation.key.clone(),
            increment_value: operation.increment_value.as_f64(),
            ttl: Some(ttl),
        });
    }
    Ok(increments)
}
