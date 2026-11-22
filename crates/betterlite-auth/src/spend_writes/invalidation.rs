use crate::spend_reseed::cache::ReseedCache;
use crate::spend_reseed::cache::dual::ReseedCacheError;

use super::batch::CounterBatch;
use super::layers::CounterLayers;

// _invalidate_spend_counter: the batch forget runs first, so no read can refill the value being cleared
pub async fn invalidate_spend_counter<Layers>(
    layers: &Layers,
    batch: &dyn CounterBatch,
    counter_key: &str,
) where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers,
{
    batch.forget(counter_key);
    if let Err(error) = layers.delete_memory(counter_key).await {
        tracing::debug!(
            %error,
            counter_key,
            "Unable to delete the stale in-memory spend counter"
        );
    }
    if layers.redis_configured()
        && let Err(error) = layers.delete_redis(counter_key).await
    {
        tracing::debug!(
            %error,
            counter_key,
            "Unable to delete the stale spend counter after increment failure"
        );
    }
}
