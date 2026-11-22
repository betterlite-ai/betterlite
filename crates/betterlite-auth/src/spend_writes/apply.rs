use std::sync::Arc;

use crate::spend_counter_batch::batch::PendingSpendIncrement;
use crate::spend_reseed::cache::ReseedCache;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::rows::SpendCounterRows;

use super::SpendCounterWrites;
use super::error::SpendWriteError;
use super::layers::CounterLayers;
use super::pipeline::increment_counters_pipeline;
use super::settlement::settle_spend_counter_increment;

// _apply_spend_counter_increments: the request batch when there is one, and a transient Redis failure settles itself
pub async fn apply_spend_counter_increments<Layers, Rows>(
    writes: &SpendCounterWrites<Layers, Rows>,
    pending: &[PendingSpendIncrement],
) -> Result<(), SpendWriteError>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + 'static,
    Rows: SpendCounterRows,
{
    if defer_spend_counter_increments(writes, pending) {
        return Ok(());
    }
    match increment_counters_pipeline(writes, pending).await {
        Ok(_counted) => Ok(()),
        Err(error) => {
            if error.is_transient() {
                tracing::debug!(
                    %error,
                    "Post-call spend counter increments skipped after a transient Redis failure"
                );
                return Ok(());
            }
            Err(error)
        }
    }
}

// _defer_spend_counter_increments: each queued increment settles into the counter's memory value when the batch lands
#[must_use]
pub fn defer_spend_counter_increments<Layers, Rows>(
    writes: &SpendCounterWrites<Layers, Rows>,
    pending: &[PendingSpendIncrement],
) -> bool
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + 'static,
    Rows: SpendCounterRows,
{
    if pending.is_empty() {
        return false;
    }
    let Some(post_call) = writes.layers.post_call() else {
        return false;
    };
    for item in pending {
        let hook = settle_spend_counter_increment(
            Arc::clone(&writes.layers),
            Arc::clone(&writes.batch),
            item.clone(),
        );
        post_call.batch.increment(
            &item.counter_key,
            item.increment.as_f64(),
            post_call.ttl,
            hook,
        );
    }
    true
}
