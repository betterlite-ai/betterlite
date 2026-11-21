use std::sync::Arc;

use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, LoopBoundSemaphore};
use betterlite_db::spend_rows::reader::SpendCounterReader;
use betterlite_db::spend_rows::rows::SpendCounterRows as DbSpendCounterRows;
use rust_decimal::Decimal;
use thiserror::Error;
use tokio_util::task::TaskTracker;

use crate::budget_counter::CounterEntityType;
use crate::budget_primitives::window_start::DurationParser;
use crate::model_budget_limiter::clock::BudgetClock;
use crate::spend_counter_batch::batch::{CounterValue, SpendCounterBatchError};

use super::cache::{ActiveSpendBatch, ReseedCache, WindowReseedCache};
use super::locks::LockRegistry;
use super::reads::{CounterDurations, cached_amount};
use super::rows::{SpendCounterRows, WindowEntity};
use super::values::{SpendValueError, aggregate_amount, stored_amount};
use super::warm::warm;
use super::windows::{WindowQuery, WindowReseed, warm_window};

#[derive(Debug, Error)]
pub enum CoalescedError<CacheError> {
    #[error(transparent)]
    Cache(CacheError),
    #[error(transparent)]
    Spend(#[from] SpendValueError),
    #[error(transparent)]
    Batch(#[from] SpendCounterBatchError),
}

pub struct SpendCounterReseed<Rows: SpendCounterRows> {
    pub rows: Option<Arc<Rows>>,
    pub locks: LockRegistry,
    pub clock: Box<dyn BudgetClock>,
    pub durations: Arc<dyn DurationParser + Send + Sync>,
    pub tasks: TaskTracker,
    pub gate: Arc<LoopBoundSemaphore>,
    pub stall_tracker: Arc<DbLookupStallTracker>,
    pub deadline_seconds: f64,
}

impl<Rows: SpendCounterRows> SpendCounterReseed<Rows> {
    // B1735's reader owns both reads: the primary entity row and the end-user budget floor
    fn reader(&self) -> SpendCounterReader<Rows>
    where
        Rows: DbSpendCounterRows,
    {
        SpendCounterReader {
            rows: self.rows.as_ref().map(Arc::clone),
            durations: Arc::new(CounterDurations {
                durations: Arc::clone(&self.durations),
                now: self.clock.now(),
            }),
            tasks: self.tasks.clone(),
            gate: Arc::clone(&self.gate),
            stall_tracker: Arc::clone(&self.stall_tracker),
            deadline_seconds: self.deadline_seconds,
        }
    }

    pub async fn from_db(&self, counter_key: &str) -> Option<Decimal>
    where
        Rows: DbSpendCounterRows,
    {
        self.reader().from_db(counter_key).await
    }

    pub async fn end_user_from_db(&self, counter_key: &str) -> Option<Decimal>
    where
        Rows: DbSpendCounterRows,
    {
        self.reader().end_user_from_db(counter_key).await
    }

    pub async fn increment_in_memory<Cache>(
        &self,
        cache: &Cache,
        counter_key: &str,
        increment: Decimal,
    ) -> Result<Option<Decimal>, Cache::Error>
    where
        Cache: ReseedCache + ?Sized,
    {
        let _guard = self.locks.hold(counter_key).await;
        cache.increment_local(counter_key, increment).await
    }

    pub async fn coalesced<Cache, Batch>(
        &self,
        cache: &Cache,
        batch: &Batch,
        counter_key: &str,
        require_cache_warm: bool,
    ) -> Result<Option<Decimal>, CoalescedError<Cache::Error>>
    where
        Rows: DbSpendCounterRows,
        Cache: ReseedCache + ?Sized,
        Batch: ActiveSpendBatch + ?Sized,
    {
        let _guard = self.locks.hold(counter_key).await;
        if let Some(amount) = cached_answer(cache, batch, counter_key).await? {
            return Ok(Some(amount));
        }
        let Some(db_spend) = self.from_db(counter_key).await else {
            return Ok(None);
        };
        warm(cache, batch, counter_key, db_spend, require_cache_warm).await
    }

    // same read precedence as coalesced, but a vanished winner's amount is added and warm failures propagate
    pub async fn coalesced_window<Cache, Batch>(
        &self,
        cache: &Cache,
        batch: &Batch,
        request: &WindowReseed<'_>,
    ) -> Result<Option<Decimal>, CoalescedError<Cache::Error>>
    where
        Cache: WindowReseedCache + ?Sized,
        Batch: ActiveSpendBatch + ?Sized,
    {
        let _guard = self.locks.hold(request.counter_key).await;
        if let Some(amount) = cached_answer(cache, batch, request.counter_key).await? {
            return Ok(Some(amount));
        }
        match self
            .window_from_db(
                request.entity_type,
                request.entity_id,
                request.window_duration,
                request.window_start,
            )
            .await?
        {
            None => Ok(None),
            Some(window_spend) => {
                warm_window(cache, batch, request.counter_key, window_spend).await
            }
        }
    }

    pub async fn window_from_table(
        &self,
        query: &WindowQuery<'_>,
        window_duration: &str,
        expected_window_start: Timestamp,
    ) -> Result<Option<Decimal>, SpendValueError> {
        let Some((rows, entity)) = self.window_source(query.entity_type) else {
            return Ok(None);
        };
        match rows
            .window_row(entity, query.entity_id, window_duration)
            .await
        {
            Ok(None) => Ok(None),
            Ok(Some(row)) => {
                if row.window_start.utc() < expected_window_start.utc() {
                    return Ok(None);
                }
                stored_amount(row.spend.as_ref()).map(Some)
            }
            Err(error) => {
                tracing::error!(%error, entity_id = query.entity_id, "SpendCounterReseed.window_from_table: failed");
                Ok(None)
            }
        }
    }

    pub async fn window_from_db(
        &self,
        entity_type: &CounterEntityType,
        entity_id: &str,
        window_duration: Option<&str>,
        window_start: Timestamp,
    ) -> Result<Option<Decimal>, SpendValueError> {
        let query = WindowQuery {
            entity_type,
            entity_id,
            start: window_start,
        };
        if let Some(duration) = window_duration {
            let from_table = self
                .window_from_table(&query, duration, window_start)
                .await?;
            if from_table.is_some() {
                return Ok(from_table);
            }
        }
        self.window_from_spend_logs(&query).await
    }

    pub async fn window_from_spend_logs(
        &self,
        query: &WindowQuery<'_>,
    ) -> Result<Option<Decimal>, SpendValueError> {
        let Some((rows, entity)) = self.window_source(query.entity_type) else {
            return Ok(None);
        };
        let found = match rows
            .window_log_rows(entity, query.entity_id, query.start.utc())
            .await
        {
            Ok(found) => found,
            Err(error) => {
                tracing::error!(%error, entity_id = query.entity_id, "SpendCounterReseed.window_from_spend_logs: failed");
                return Ok(None);
            }
        };
        found.first().map_or(Ok(Some(Decimal::ZERO)), |row| {
            aggregate_amount(row).map(Some)
        })
    }

    fn window_source(&self, entity_type: &CounterEntityType) -> Option<(&Rows, WindowEntity)> {
        let rows = self.rows.as_deref()?;
        let entity = WindowEntity::from_counter_type(entity_type)?;
        Some((rows, entity))
    }
}

// a batched amount answers first, and a batched null keeps the per-pod memory from answering
async fn cached_answer<Cache, Batch>(
    cache: &Cache,
    batch: &Batch,
    counter_key: &str,
) -> Result<Option<Decimal>, CoalescedError<Cache::Error>>
where
    Cache: ReseedCache + ?Sized,
    Batch: ActiveSpendBatch + ?Sized,
{
    match batch.read(counter_key).await? {
        Some(CounterValue::Amount(amount)) => Ok(Some(amount)),
        Some(CounterValue::Missing) => cached_amount(cache, true, counter_key).await,
        None => cached_amount(cache, false, counter_key).await,
    }
}
