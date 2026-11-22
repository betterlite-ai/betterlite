use std::sync::Arc;

use crate::spend_counter_batch::batch::PendingSpendIncrement;
use crate::spend_reseed::cache::ReseedCache;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::rows::SpendCounterRows;

use super::SpendCounterWrites;
use super::error::SpendWriteError;
use super::layers::CounterLayers;
use super::settlement::settle_spend_counter_increment;

impl<Layers, Rows> SpendCounterWrites<Layers, Rows>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + 'static,
    Rows: SpendCounterRows,
{
    // _apply_spend_counter_increments: the request batch when there is one, and a transient Redis failure settles itself
    pub async fn apply_spend_counter_increments(
        &self,
        pending: &[PendingSpendIncrement],
    ) -> Result<(), SpendWriteError> {
        if self.defer_spend_counter_increments(pending) {
            return Ok(());
        }
        match self.increment_counters_pipeline(pending).await {
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
    pub fn defer_spend_counter_increments(&self, pending: &[PendingSpendIncrement]) -> bool {
        if pending.is_empty() {
            return false;
        }
        let Some(post_call) = self.layers.post_call() else {
            return false;
        };
        for item in pending {
            let hook = settle_spend_counter_increment(
                Arc::clone(&self.layers),
                Arc::clone(&self.batch),
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
}
