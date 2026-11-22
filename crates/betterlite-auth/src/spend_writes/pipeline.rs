use rust_decimal::Decimal;

use crate::spend_counter_batch::batch::PendingSpendIncrement;
use crate::spend_reseed::cache::ReseedCache;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::rows::SpendCounterRows;

use super::SpendCounterWrites;
use super::error::SpendWriteError;
use super::invalidation::invalidate_spend_counter;
use super::layers::CounterLayers;

// increment_spend_counters_pipeline: the validating pipeline admission takes, clearing every counter first
pub async fn increment_counters_pipeline<Layers, Rows>(
    writes: &SpendCounterWrites<Layers, Rows>,
    pending: &[PendingSpendIncrement],
) -> Result<Vec<Option<Decimal>>, SpendWriteError>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers,
    Rows: SpendCounterRows,
{
    if !writes.layers.redis_configured() {
        return run_spend_counter_pipeline(writes, pending).await;
    }
    match run_spend_counter_pipeline(writes, pending).await {
        Ok(counted) => Ok(counted),
        Err(error) => {
            for item in pending {
                invalidate_spend_counter(&*writes.layers, &*writes.batch, &item.counter_key).await;
            }
            Err(error)
        }
    }
}

// run_spend_counter_pipeline: one INCRBYFLOAT+EXPIRE per counter, and a failure leaves the cleanup to the caller
pub async fn run_spend_counter_pipeline<Layers, Rows>(
    writes: &SpendCounterWrites<Layers, Rows>,
    pending: &[PendingSpendIncrement],
) -> Result<Vec<Option<Decimal>>, SpendWriteError>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers,
    Rows: SpendCounterRows,
{
    if pending.is_empty() {
        return Ok(Vec::new());
    }
    if !writes.layers.redis_configured() {
        let mut counted = Vec::with_capacity(pending.len());
        for item in pending {
            let amount = writes
                .reseed
                .increment_in_memory(&*writes.layers, &item.counter_key, item.increment)
                .await?;
            counted.push(amount);
        }
        return Ok(counted);
    }
    let amounts = writes.layers.increment_redis_pipeline(pending).await?;
    let mut counted = Vec::with_capacity(amounts.len());
    for (item, amount) in pending.iter().zip(amounts) {
        writes.layers.memory_set(&item.counter_key, amount).await?;
        writes.batch.record(&item.counter_key, amount);
        counted.push(Some(amount));
    }
    Ok(counted)
}
