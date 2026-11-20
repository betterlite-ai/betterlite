use std::error::Error as StdError;
use std::future::Future;
use std::sync::Arc;

use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::database_failures::deadline::{DbLookupError, LookupRequest};
use betterlite_db::database_failures::gate::{DbLookupStallTracker, LoopBoundSemaphore};
use rust_decimal::Decimal;
use thiserror::Error;
use tokio_util::task::TaskTracker;

use crate::budget_counter::CounterEntityType;
use crate::budget_primitives::window_start::DurationParser;
use crate::model_budget_limiter::clock::BudgetClock;
use crate::spend_counter_batch::batch::{CounterValue, SpendCounterBatchError};

use super::cache::{ActiveSpendBatch, ReseedCache, WindowReseedCache};
use super::locks::LockRegistry;
use super::reads::{bounded_row, cached_amount, end_user_lookup, gated_primary, primary_lookup};
use super::rows::{LookupAnswer, OwnedLookup, SpendCounterRows, SpendRow, WindowEntity};
use super::values::{SpendValueError, aggregate_amount, stored_amount};
use super::warm::{warm, warm_window};
use super::windows::WindowQuery;

pub const SPEND_COUNTER_LOOKUP: &str = "spend_counter";
pub const END_USER_SPEND_LOOKUP: &str = "end_user_spend";

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
    pub durations: Box<dyn DurationParser + Send + Sync>,
    pub tasks: TaskTracker,
    pub gate: Arc<LoopBoundSemaphore>,
    pub stall_tracker: Arc<DbLookupStallTracker>,
    pub deadline_seconds: f64,
}

impl<Rows: SpendCounterRows> SpendCounterReseed<Rows> {
    pub async fn from_db(&self, counter_key: &str) -> Result<Option<Decimal>, SpendValueError> {
        let Some(rows) = self.rows.as_ref().map(Arc::clone) else {
            return Ok(None);
        };
        let Some(lookup) = primary_lookup(counter_key, self.durations.as_ref(), self.clock.now())
        else {
            return Ok(None);
        };
        let owned = OwnedLookup::from(lookup);
        let gate = Arc::clone(&self.gate);
        let request = async move {
            let answer = gated_primary(gate.as_ref(), rows.as_ref(), owned).await;
            Ok::<_, DbLookupError>(answer)
        };
        self.bounded_amount(request, SPEND_COUNTER_LOOKUP, counter_key)
            .await
    }

    pub async fn end_user_from_db(
        &self,
        counter_key: &str,
    ) -> Result<Option<Decimal>, SpendValueError> {
        let Some(rows) = self.rows.as_ref().map(Arc::clone) else {
            return Ok(None);
        };
        let Some(lookup) = end_user_lookup(counter_key, self.durations.as_ref(), self.clock.now())
        else {
            return Ok(None);
        };
        let owned = OwnedLookup::from(lookup);
        // the floor read carries no db lookup gate: the bounded lookup alone guards it
        let request = async move {
            let answer = match rows.spend_row(owned.borrowed()).await {
                Ok(row) => LookupAnswer::Row(row),
                Err(error) => LookupAnswer::Failed(error),
            };
            Ok::<_, DbLookupError>(answer)
        };
        self.bounded_amount(request, END_USER_SPEND_LOOKUP, counter_key)
            .await
    }

    async fn bounded_amount<Failure, Lookup>(
        &self,
        lookup: Lookup,
        name: &str,
        counter_key: &str,
    ) -> Result<Option<Decimal>, SpendValueError>
    where
        Failure: StdError + Send + Sync + 'static,
        Lookup: Future<Output = Result<LookupAnswer<SpendRow, Failure>, DbLookupError>>
            + Send
            + 'static,
    {
        let Some(row) =
            bounded_row(lookup, self.lookup_request(name), &self.tasks, counter_key).await
        else {
            return Ok(None);
        };
        row.map(|stored| stored_amount(stored.spend.as_ref()))
            .transpose()
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
        Cache: ReseedCache + ?Sized,
        Batch: ActiveSpendBatch + ?Sized,
    {
        let _guard = self.locks.hold(counter_key).await;
        let batched = batch.read(counter_key).await?;
        if let Some(CounterValue::Amount(amount)) = batched {
            return Ok(Some(amount));
        }
        if let Some(amount) = cached_amount(cache, batched.is_some(), counter_key).await? {
            return Ok(Some(amount));
        }
        let Some(db_spend) = self.from_db(counter_key).await? else {
            return Ok(None);
        };
        warm(cache, batch, counter_key, db_spend, require_cache_warm).await
    }

    // the same read precedence as coalesced, but the warm step adds a vanished winner's amount
    // and every warm failure propagates: there is no require_cache_warm at this admission point
    pub async fn coalesced_window<Cache, Batch>(
        &self,
        cache: &Cache,
        batch: &Batch,
        counter_key: &str,
        entity_type: &CounterEntityType,
        entity_id: &str,
        window_duration: Option<&str>,
        window_start: Timestamp,
    ) -> Result<Option<Decimal>, CoalescedError<Cache::Error>>
    where
        Cache: WindowReseedCache + ?Sized,
        Batch: ActiveSpendBatch + ?Sized,
    {
        let _guard = self.locks.hold(counter_key).await;
        let batched = batch.read(counter_key).await?;
        if let Some(CounterValue::Amount(amount)) = batched {
            return Ok(Some(amount));
        }
        if let Some(amount) = cached_amount(cache, batched.is_some(), counter_key).await? {
            return Ok(Some(amount));
        }
        let Some(window_spend) = self
            .window_from_db(entity_type, entity_id, window_duration, window_start)
            .await?
        else {
            return Ok(None);
        };
        warm_window(cache, batch, counter_key, window_spend).await
    }

    fn lookup_request<'request>(&'request self, name: &'request str) -> LookupRequest<'request> {
        LookupRequest {
            name,
            deadline_seconds: None,
            default_deadline_seconds: self.deadline_seconds,
            tracker: self.stall_tracker.as_ref(),
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
