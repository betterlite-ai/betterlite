use rust_decimal::Decimal;

use crate::spend_counter_batch::batch::PendingSpendIncrement;
use crate::spend_reseed::cache::ReseedCache;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::rows::SpendCounterRows;

use super::SpendCounterWrites;
use super::error::SpendWriteError;
use super::invalidation::invalidate_spend_counter;
use super::layers::CounterLayers;

impl<Layers, Rows> SpendCounterWrites<Layers, Rows>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers,
    Rows: SpendCounterRows,
{
    // increment_spend_counters_pipeline: the validating pipeline admission takes, clearing every counter first
    pub async fn increment_counters_pipeline(
        &self,
        pending: &[PendingSpendIncrement],
    ) -> Result<Vec<Option<Decimal>>, SpendWriteError> {
        if !self.layers.redis_configured() {
            return self.run_spend_counter_pipeline(pending).await;
        }
        match self.run_spend_counter_pipeline(pending).await {
            Ok(counted) => Ok(counted),
            Err(error) => {
                for item in pending {
                    invalidate_spend_counter(&*self.layers, &*self.batch, &item.counter_key).await;
                }
                Err(error)
            }
        }
    }

    // run_spend_counter_pipeline: one INCRBYFLOAT+EXPIRE per counter, and a failure leaves the cleanup to the caller
    pub async fn run_spend_counter_pipeline(
        &self,
        pending: &[PendingSpendIncrement],
    ) -> Result<Vec<Option<Decimal>>, SpendWriteError> {
        if pending.is_empty() {
            return Ok(Vec::new());
        }
        if !self.layers.redis_configured() {
            let mut counted = Vec::with_capacity(pending.len());
            for item in pending {
                let amount = self
                    .reseed
                    .increment_in_memory(&*self.layers, &item.counter_key, item.increment)
                    .await?;
                counted.push(amount);
            }
            return Ok(counted);
        }
        let amounts = self.layers.increment_redis_pipeline(pending).await?;
        let mut counted = Vec::with_capacity(amounts.len());
        for (item, amount) in pending.iter().zip(amounts) {
            self.layers.memory_set(&item.counter_key, amount).await?;
            self.batch.record(&item.counter_key, amount);
            counted.push(Some(amount));
        }
        Ok(counted)
    }
}
