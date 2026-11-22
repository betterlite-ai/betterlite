use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::spend_rows::rows::SpendCounterRows as DbSpendCounterRows;
use chrono::DateTime;
use rust_decimal::Decimal;

use crate::budget_counter::BudgetCounter;
use crate::reservation_settlement::backend::SpendCounterBackend;
use crate::reservation_settlement::error::SettlementError;
use crate::spend_counter_batch::batch::CounterValue;
use crate::spend_counter_batch::batch::PendingSpendIncrement;
use crate::spend_counter_batch::batch::SpendCounterBatchError;
use crate::spend_reads::SpendReads;
use crate::spend_reads::cache::SpendReadCache;
use crate::spend_reseed::cache::ActiveSpendBatch;
use crate::spend_reseed::cache::ReseedCache;
use crate::spend_reseed::cache::WindowReseedCache;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::rows::SpendCounterRows;
use crate::spend_reseed::windows::WindowReseed;
use crate::spend_writes::batch::CounterBatch;
use crate::spend_writes::error::SpendWriteError;
use crate::spend_writes::increment::increment_spend_counter;
use crate::spend_writes::initialization::get_source_cache_base_spend;
use crate::spend_writes::layers::CounterLayers;
use crate::spend_writes::pipeline::run_spend_counter_pipeline;
use crate::spend_writes::warm::is_spend_counter_cache_warm;

use super::counters::AdmissionCounters;

// the writes' dyn batch implements ActiveSpendBatch only for the 'static object, which rustc cannot generalize in the reservation's Send future (rust-lang/rust#130113); an owned newtype covers every batch
struct SeedBatch(Arc<dyn CounterBatch>);

impl ActiveSpendBatch for SeedBatch {
    async fn read(
        &self,
        counter_key: &str,
    ) -> Result<Option<CounterValue>, SpendCounterBatchError> {
        self.0.as_ref().read(counter_key).await
    }

    fn record(&self, counter_key: &str, value: Decimal) {
        self.0.as_ref().record(counter_key, value);
    }
}

// spend_counter_cache and the request's spend_counter_batch_scope as admission reaches them
#[async_trait]
impl<Layers, Rows> AdmissionCounters for SpendCounterBackend<'_, Layers, Rows>
where
    Layers:
        ReseedCache<Error = ReseedCacheError> + CounterLayers + SpendReadCache + WindowReseedCache,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    async fn seed_source(
        &self,
        counter_key: &str,
        source_cache_key: &str,
    ) -> Result<(), SpendWriteError> {
        if is_spend_counter_cache_warm(self.writes, counter_key).await? {
            return Ok(());
        }
        let seed_batch = SeedBatch(Arc::clone(&self.writes.batch));
        let seeded = self
            .writes
            .reseed
            .coalesced(&*self.writes.layers, &seed_batch, counter_key, true)
            .await?;
        if seeded.is_some() {
            return Ok(());
        }
        let base_spend = get_source_cache_base_spend(self.writes, &[source_cache_key]).await?;
        if base_spend > Decimal::ZERO {
            let _counted = increment_spend_counter(self.writes, counter_key, base_spend).await?;
        }
        Ok(())
    }

    async fn seed_window(&self, counter: &BudgetCounter) -> Result<bool, SpendWriteError> {
        let Some(start) = counter.window_start.and_then(window_timestamp) else {
            return Ok(false);
        };
        let request = WindowReseed {
            counter_key: &counter.counter_key,
            entity_type: &counter.entity_type,
            entity_id: counter
                .spend_log_entity_id
                .as_deref()
                .unwrap_or(&counter.entity_id),
            window_duration: counter.window_duration.as_deref(),
            window_start: start,
        };
        if is_spend_counter_cache_warm(self.writes, request.counter_key).await? {
            return Ok(true);
        }
        let seed_batch = SeedBatch(Arc::clone(&self.writes.batch));
        let window_spend = self
            .writes
            .reseed
            .coalesced_window(&*self.writes.layers, &seed_batch, &request)
            .await?;
        if window_spend.is_none() {
            tracing::warn!(
                counter_key = request.counter_key,
                "Skipping cold spend counter seed because window spend could not be loaded"
            );
            return Ok(false);
        }
        Ok(true)
    }

    async fn current_spend(&self, counter_key: &str, fallback_spend: Decimal) -> Decimal {
        let reads = SpendReads {
            reseed: self.writes.reseed.as_ref(),
            cache: self.writes.layers.as_ref(),
            batch: Some(self.scope),
            fail_closed_budget_enforcement: false,
        };
        let (amount, _authoritative) = reads
            .read_spend_counter_estimate(counter_key, fallback_spend)
            .await;
        amount
    }

    async fn reserve_pipeline(
        &self,
        pending: &[PendingSpendIncrement],
    ) -> Result<Vec<Option<Decimal>>, SettlementError> {
        let counted = run_spend_counter_pipeline(self.writes, pending).await?;
        Ok(counted)
    }
}

// a window start chrono cannot place on the calendar names no spend row
fn window_timestamp(start: SystemTime) -> Option<Timestamp> {
    let since_epoch = start.duration_since(UNIX_EPOCH).ok()?;
    let seconds = i64::try_from(since_epoch.as_secs()).ok()?;
    let moment = DateTime::from_timestamp(seconds, since_epoch.subsec_nanos())?;
    Some(Timestamp::Aware(moment.fixed_offset()))
}
