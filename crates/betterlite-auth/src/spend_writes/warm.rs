use crate::spend_counter_batch::batch::CounterValue;
use crate::spend_reseed::cache::ReseedCache;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::rows::SpendCounterRows;
use crate::spend_reseed::values::ReseedCacheValue;

use super::SpendCounterWrites;
use super::error::SpendWriteError;
use super::layers::CounterLayers;

pub async fn is_spend_counter_cache_warm<Layers, Rows>(
    writes: &SpendCounterWrites<Layers, Rows>,
    counter_key: &str,
) -> Result<bool, SpendWriteError>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers,
    Rows: SpendCounterRows,
{
    match writes.batch.read(counter_key).await? {
        Some(CounterValue::Amount(amount)) => {
            writes
                .layers
                .set_memory_value(counter_key, ReseedCacheValue::Number(amount))
                .await?;
            return Ok(true);
        }
        // a batched miss is Redis's own answer, so the per-pod memory tier cannot overrule it
        Some(CounterValue::Missing) => return Ok(false),
        None => {}
    }
    if writes.layers.redis_configured() {
        match writes.layers.redis_get(counter_key).await {
            Ok(Some(value)) => {
                writes.layers.set_memory_value(counter_key, value).await?;
                return Ok(true);
            }
            // a clean Redis miss is Redis's own answer, so the per-pod memory tier cannot overrule it
            Ok(None) => return Ok(false),
            Err(error) => tracing::debug!(
                %error,
                counter_key,
                "Unable to read the Redis spend counter before initialization, falling back to in-memory"
            ),
        }
    }
    Ok(writes.layers.memory_get(counter_key).await?.is_some())
}
