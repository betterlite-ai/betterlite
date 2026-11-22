use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::spend_rows::rows::SpendCounterRows as DbSpendCounterRows;
use betterlite_redis::redis_cache::RedisCache;
use rust_decimal::Decimal;

use crate::spend_counter_batch::batch::PendingSpendIncrement;
use crate::spend_counter_batch::scope::SpendCounterScope;
use crate::spend_reads::SpendReads;
use crate::spend_reads::cache::SpendReadCache;
use crate::spend_reseed::cache::ReseedCache;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::rows::SpendCounterRows;
use crate::spend_writes::SpendCounterWrites;
use crate::spend_writes::increment::increment_spend_counter;
use crate::spend_writes::invalidation::invalidate_spend_counter;
use crate::spend_writes::layers::CounterLayers;
use crate::spend_writes::pipeline::increment_counters_pipeline;

use super::counters::ScopeHold;
use super::counters::SettlementCounters;
use super::error::CounterReadError;
use super::error::SettlementError;

// spend_counter_cache and the request's spend_counter_batch_scope as the settlement code reaches them
pub struct SpendCounterBackend<'a, Layers, Rows>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + SpendReadCache,
    Rows: SpendCounterRows,
{
    pub writes: &'a SpendCounterWrites<Layers, Rows>,
    pub scope: &'a SpendCounterScope,
    pub redis: Option<&'a Arc<RedisCache>>,
}

impl<Layers, Rows> SpendCounterBackend<'_, Layers, Rows>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + SpendReadCache,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    // the request's batch stands in front of the cache tiers, so one scope answers every counter read
    fn reads(&self) -> SpendReads<'_, Rows, Layers, SpendCounterScope> {
        SpendReads {
            reseed: self.writes.reseed.as_ref(),
            cache: self.writes.layers.as_ref(),
            batch: Some(self.scope),
            fail_closed_budget_enforcement: false,
        }
    }
}

#[async_trait]
impl<Layers, Rows> SettlementCounters for SpendCounterBackend<'_, Layers, Rows>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + SpendReadCache,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    fn enter_counters_batch_scope(
        &self,
        counter_keys: &BTreeSet<String>,
    ) -> Box<dyn ScopeHold + Send + '_> {
        Box::new(self.scope.enter(self.redis, counter_keys))
    }

    async fn read_cache_value(
        &self,
        counter_key: &str,
    ) -> Result<Option<Decimal>, CounterReadError> {
        let (value, _authoritative) = self
            .reads()
            .read_spend_counter_cache_value(counter_key)
            .await;
        Ok(value)
    }

    async fn reseed_from_db(&self, counter_key: &str) -> bool {
        self.reads().reseed_spend_counter_from_db(counter_key).await
    }

    async fn increment_counter(
        &self,
        counter_key: &str,
        increment: Decimal,
    ) -> Result<(), SettlementError> {
        let _counted = increment_spend_counter(self.writes, counter_key, increment).await?;
        Ok(())
    }

    async fn increment_pipeline(
        &self,
        pending: &[PendingSpendIncrement],
    ) -> Result<(), SettlementError> {
        let _counted = increment_counters_pipeline(self.writes, pending).await?;
        Ok(())
    }

    async fn invalidate(&self, counter_key: &str) {
        invalidate_spend_counter(
            self.writes.layers.as_ref(),
            self.writes.batch.as_ref(),
            counter_key,
        )
        .await;
    }
}
