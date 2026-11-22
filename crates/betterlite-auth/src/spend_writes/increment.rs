use rust_decimal::Decimal;

use crate::spend_reseed::cache::ReseedCache;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::rows::SpendCounterRows;

use super::SpendCounterWrites;
use super::error::SpendWriteError;
use super::invalidation::invalidate_spend_counter;
use super::layers::CounterLayers;

// increment_spend_counter and _increment_spend_counter_cache: the one write every spend domain shares
pub async fn increment_spend_counter<Layers, Rows>(
    writes: &SpendCounterWrites<Layers, Rows>,
    counter_key: &str,
    increment: Decimal,
) -> Result<Option<Decimal>, SpendWriteError>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers,
    Rows: SpendCounterRows,
{
    if !writes.layers.redis_configured() {
        let counted = writes
            .reseed
            .increment_in_memory(&*writes.layers, counter_key, increment)
            .await?;
        return Ok(counted);
    }
    let counted = match writes.layers.increment_redis(counter_key, increment).await {
        Ok(counted) => counted,
        Err(error) => {
            invalidate_spend_counter(&*writes.layers, &*writes.batch, counter_key).await;
            return Err(SpendWriteError::from(error));
        }
    };
    writes.layers.memory_set(counter_key, counted).await?;
    writes.batch.record(counter_key, counted);
    Ok(Some(counted))
}
