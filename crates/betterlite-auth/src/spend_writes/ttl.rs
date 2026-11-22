use crate::spend_reseed::cache::ReseedCache;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::rows::SpendCounterRows;

use super::SpendCounterWrites;
use super::layers::CounterLayers;

// refresh_spend_counter_ttl: the value gates admission, the expiry is defensive, so a failure is a false
pub async fn refresh_spend_counter_ttl<Layers, Rows>(
    writes: &SpendCounterWrites<Layers, Rows>,
    counter_key: &str,
) -> bool
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers,
    Rows: SpendCounterRows,
{
    if !writes.layers.redis_configured() {
        return false;
    }
    match writes.layers.refresh_redis_ttl(counter_key).await {
        Ok(refreshed) => refreshed,
        Err(error) => {
            tracing::debug!(
                %error,
                counter_key,
                "spend counter TTL refresh skipped for the counter"
            );
            false
        }
    }
}
