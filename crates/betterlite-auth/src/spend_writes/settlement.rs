use std::sync::Arc;

use betterlite_redis::batch::error::BatchError;
use betterlite_redis::batch::result::{Settled, SettledFollowUp, SettledHook};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

use crate::spend_counter_batch::batch::PendingSpendIncrement;
use crate::spend_reseed::cache::ReseedCache;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::values::ReseedCacheValue;

use super::batch::CounterBatch;
use super::invalidation::invalidate_spend_counter;
use super::layers::CounterLayers;

// _settle_spend_counter_increment: the post-call hook that turns the pipeline's answer into the counter's next state
pub fn settle_spend_counter_increment<Layers>(
    layers: Arc<Layers>,
    batch: Arc<dyn CounterBatch>,
    item: PendingSpendIncrement,
) -> SettledHook<f64>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + 'static,
{
    Arc::new(move |settled: &Settled<f64>| {
        let settled = settled.clone();
        let item = item.clone();
        let layers = Arc::clone(&layers);
        let batch = Arc::clone(&batch);
        let follow: SettledFollowUp =
            Box::pin(async move { settle_spend_counter(&layers, &*batch, &item, settled).await });
        Some(follow)
    })
}

async fn settle_spend_counter<Layers>(
    layers: &Layers,
    batch: &dyn CounterBatch,
    item: &PendingSpendIncrement,
    settled: Settled<f64>,
) where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers,
{
    match settled {
        Ok(value) => {
            let Some(amount) = Decimal::from_f64(value) else {
                tracing::debug!(
                    counter_key = %item.counter_key,
                    value,
                    "Settled spend counter has no decimal form"
                );
                return;
            };
            if let Err(error) = layers
                .set_memory_value(&item.counter_key, ReseedCacheValue::Number(amount))
                .await
            {
                tracing::debug!(
                    %error,
                    counter_key = %item.counter_key,
                    "Unable to store the settled spend counter in memory"
                );
            }
            batch.record(&item.counter_key, amount);
        }
        Err(error) => {
            if matches!(*error, BatchError::Cancelled) {
                settle_cancelled(layers, item).await;
                return;
            }
            tracing::warn!(
                %error,
                counter_key = %item.counter_key,
                "Spend counter increment did not land in the post-call pipeline; invalidating it"
            );
            invalidate_spend_counter(layers, batch, &item.counter_key).await;
        }
    }
}

// a cancelled pipeline never applied the increment, so the value already in memory keeps its own count
async fn settle_cancelled<Layers>(layers: &Layers, item: &PendingSpendIncrement)
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers,
{
    match layers.memory_get(&item.counter_key).await {
        Ok(Some(_present)) => {
            if let Err(error) = layers
                .increment_local(&item.counter_key, item.increment)
                .await
            {
                tracing::debug!(
                    %error,
                    counter_key = %item.counter_key,
                    "Unable to apply the cancelled spend counter increment in memory"
                );
            }
        }
        Ok(None) => {}
        Err(error) => tracing::debug!(
            %error,
            counter_key = %item.counter_key,
            "Unable to read the in-memory spend counter after a cancelled increment"
        ),
    }
}
