pub mod dual;

use std::sync::Arc;

use async_trait::async_trait;
use betterlite_redis::batch::result::SettledHook;
use rust_decimal::Decimal;

use crate::spend_counter_batch::batch::PendingSpendIncrement;
use crate::spend_reseed::values::ReseedCacheValue;

use super::error::LayersError;

// the request's post-call Redis batch, and the default TTL LiteLLM puts on each queued increment
pub struct PostCallBatch {
    pub batch: Arc<dyn CounterWriteBatch>,
    pub ttl: i64,
}

// a RedisBatch narrowed to the one operation the spend counter deferral queues
pub trait CounterWriteBatch: Send + Sync {
    fn increment(&self, counter_key: &str, increment: f64, ttl: i64, hook: SettledHook<f64>);
}

// the cache operations spend counters write through beyond the reads B1736's ReseedCache already takes
#[async_trait]
pub trait CounterLayers: Send + Sync {
    async fn increment_redis(
        &self,
        counter_key: &str,
        increment: Decimal,
    ) -> Result<Decimal, LayersError>;

    async fn refresh_redis_ttl(&self, counter_key: &str) -> Result<bool, LayersError>;

    async fn delete_redis(&self, counter_key: &str) -> Result<(), LayersError>;

    async fn delete_memory(&self, counter_key: &str) -> Result<(), LayersError>;

    async fn set_memory_value(
        &self,
        counter_key: &str,
        value: ReseedCacheValue,
    ) -> Result<(), LayersError>;

    async fn increment_redis_pipeline(
        &self,
        pending: &[PendingSpendIncrement],
    ) -> Result<Vec<Decimal>, LayersError>;

    fn post_call(&self) -> Option<PostCallBatch>;
}
