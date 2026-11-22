use std::fmt::Display;
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
        let follow: SettledFollowUp = Box::pin(async move {
            let settlement = Settlement {
                layers: &*layers,
                batch: &*batch,
                item: &item,
            };
            settlement.settle(settled).await;
        });
        Some(follow)
    })
}

struct Settlement<'a, Layers> {
    layers: &'a Layers,
    batch: &'a dyn CounterBatch,
    item: &'a PendingSpendIncrement,
}

impl<Layers> Settlement<'_, Layers>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers,
{
    async fn settle(&self, settled: Settled<f64>) {
        match settled {
            Ok(value) => self.store(value).await,
            Err(error) => self.failed(&error).await,
        }
    }

    async fn store(&self, value: f64) {
        let Some(amount) = Decimal::from_f64(value) else {
            tracing::debug!(
                counter_key = %self.item.counter_key,
                value,
                "Settled spend counter has no decimal form"
            );
            return;
        };
        if let Err(error) = self
            .layers
            .set_memory_value(&self.item.counter_key, ReseedCacheValue::Number(amount))
            .await
        {
            debug_failure(
                &error,
                &self.item.counter_key,
                "Unable to store the settled spend counter in memory",
            );
        }
        self.batch.record(&self.item.counter_key, amount);
    }

    async fn failed(&self, error: &BatchError) {
        if matches!(error, BatchError::Cancelled) {
            return self.cancelled().await;
        }
        tracing::warn!(
            %error,
            counter_key = %self.item.counter_key,
            "Spend counter increment did not land in the post-call pipeline; invalidating it"
        );
        invalidate_spend_counter(self.layers, self.batch, &self.item.counter_key).await;
    }

    // a cancelled pipeline never applied the increment, so the value already in memory keeps its own count
    async fn cancelled(&self) {
        match self.layers.memory_get(&self.item.counter_key).await {
            Ok(Some(_present)) => {
                if let Err(error) = self
                    .layers
                    .increment_local(&self.item.counter_key, self.item.increment)
                    .await
                {
                    debug_failure(
                        &error,
                        &self.item.counter_key,
                        "Unable to apply the cancelled spend counter increment in memory",
                    );
                }
            }
            Ok(None) => {}
            Err(error) => debug_failure(
                &error,
                &self.item.counter_key,
                "Unable to read the in-memory spend counter after a cancelled increment",
            ),
        }
    }
}

fn debug_failure<T: Display>(error: &T, counter_key: &str, message: &str) {
    tracing::debug!(
        %error,
        counter_key = %counter_key,
        message = %message,
    );
}
