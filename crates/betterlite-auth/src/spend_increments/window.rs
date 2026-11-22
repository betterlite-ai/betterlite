use std::future::Future;
use std::pin::Pin;

use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::spend_rows::rows::SpendCounterRows as DbSpendCounterRows;
use betterlite_db::window_queue::queue::WindowSpendUpdateQueue;
use betterlite_db::window_queue::stamp::{
    SystemZone, WindowIncrement, WindowStamp, build_window_spend_transaction,
};
use betterlite_db::window_queue::{WindowQueueError, WindowSpendTransaction};
use chrono::{DateTime, FixedOffset, Utc};
use rust_decimal::Decimal;

use crate::budget_counter::CounterEntityType;
use crate::budget_primitives::window::{BudgetWindow, coerce_window};
use crate::budget_primitives::window_start::{DurationParser, budget_window_start};
use crate::reservation_settlement::counters::SettlementCounters;
use crate::spend_counter_batch::batch::PendingSpendIncrement;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::cache::{ReseedCache, WindowReseedCache};
use crate::spend_reseed::rows::SpendCounterRows;
use crate::spend_reseed::windows::WindowReseed;
use crate::spend_writes::SpendCounterWrites;
use crate::spend_writes::initialization::ensure_window_seeded;
use crate::spend_writes::layers::CounterLayers;

use super::SpendIncrementServices;
use super::error::SpendIncrementError;
use super::scopes::{ScopeCharge, ScopeOutcome, scope_outcome};

pub type WindowQueueFuture<'queue> =
    Pin<Box<dyn Future<Output = Result<(), WindowQueueError>> + Send + 'queue>>;

// WindowSpendUpdateQueue.add_update behind a port: the fan-out charges whatever queue the wiring built, and a test can fail one enqueue on purpose
pub trait WindowSpendQueue: Send + Sync {
    fn enqueue(&self, transaction: WindowSpendTransaction) -> WindowQueueFuture<'_>;
}

impl WindowSpendQueue for WindowSpendUpdateQueue {
    fn enqueue(&self, transaction: WindowSpendTransaction) -> WindowQueueFuture<'_> {
        Box::pin(self.add_update(transaction))
    }
}

// a key or team scope: its counter, the cache key its windows come from, and the row identity they charge
pub(super) struct WindowScope {
    pub(super) counter_key: String,
    pub(super) source_key: String,
    pub(super) entity_type: CounterEntityType,
    pub(super) entity_id: String,
}

// one budget_limits entry, with the counter key and window start the request charges it under
pub(super) struct WindowCharge {
    pub(super) counter_key: String,
    pub(super) entity_type: CounterEntityType,
    pub(super) entity_id: String,
    pub(super) window_duration: String,
    pub(super) window_start: Option<DateTime<FixedOffset>>,
    pub(super) reset_at: bool,
    pub(super) cost: Decimal,
    pub(super) request_started_at: Option<DateTime<Utc>>,
}

// one window's share of the scope: its pending increment, and the row the window spend table owes even when the counter is reserved
pub(super) async fn window_outcome<Counters, Layers, Rows>(
    facts: &WindowCharge,
    charge: &ScopeCharge,
    services: &SpendIncrementServices<'_, Counters, Layers, Rows>,
) -> ScopeOutcome
where
    Counters: SettlementCounters,
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + WindowReseedCache + 'static,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    let prepared = if charge.reserved.contains(&facts.counter_key) {
        Ok(None)
    } else {
        prepare_window_increment(facts, services.writes).await
    };
    match prepared {
        Ok(pending) => {
            enqueue_window_row(facts, services.window_queue).await;
            scope_outcome(Ok(pending))
        }
        Err(error) => ScopeOutcome {
            pending: Vec::new(),
            error: Some(error),
        },
    }
}

// _prepare_window_spend_counter_increment: an unmappable window start warns and charges nothing, and a cold unreadable window skips quietly
pub(super) async fn prepare_window_increment<Layers, Rows>(
    charge: &WindowCharge,
    writes: &SpendCounterWrites<Layers, Rows>,
) -> Result<Option<PendingSpendIncrement>, SpendIncrementError>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers + WindowReseedCache,
    Rows: SpendCounterRows,
{
    let Some(window_start) = charge.window_start else {
        let counter_key = charge.counter_key.as_str();
        tracing::warn!("Skipping spend counter increment for invalid budget window {counter_key}");
        return Ok(None);
    };
    let seeded = ensure_window_seeded(
        writes,
        &WindowReseed {
            counter_key: &charge.counter_key,
            entity_type: &charge.entity_type,
            entity_id: &charge.entity_id,
            window_duration: Some(&charge.window_duration),
            window_start: Timestamp::Aware(window_start),
        },
    )
    .await?;
    if !seeded {
        return Ok(None);
    }
    Ok(Some(PendingSpendIncrement {
        counter_key: charge.counter_key.clone(),
        increment: charge.cost,
    }))
}

// _enqueue_window_spend_row_update: a window without reset_at slides with the clock, and a queue failure never fails the callback
pub(super) async fn enqueue_window_row(charge: &WindowCharge, queue: &dyn WindowSpendQueue) {
    let Some(transaction) = window_row_transaction(charge) else {
        return;
    };
    if let Err(error) = queue.enqueue(transaction).await {
        tracing::debug!(
            %error,
            entity_type = charge.entity_type.wire_value(),
            entity_id = charge.entity_id.as_str(),
            window_duration = charge.window_duration.as_str(),
            "Unable to enqueue budget window spend update"
        );
    }
}

// one window's queued increment, or none when its start is unknown or its reset_at slides with the clock
fn window_row_transaction(charge: &WindowCharge) -> Option<WindowSpendTransaction> {
    let window_start = charge.window_start?;
    if !charge.reset_at {
        return None;
    }
    let increment = WindowIncrement {
        entity_type: charge.entity_type.wire_value(),
        entity_id: &charge.entity_id,
        window_duration: &charge.window_duration,
        window_start: WindowStamp::Aware(window_start),
        spend: charge.cost,
        started_at: charge
            .request_started_at
            .map(|started| WindowStamp::Aware(started.fixed_offset())),
    };
    match build_window_spend_transaction(&increment, &SystemZone) {
        Ok(transaction) => Some(transaction),
        Err(error) => {
            tracing::debug!(
                %error,
                entity_type = charge.entity_type.wire_value(),
                entity_id = charge.entity_id.as_str(),
                window_duration = charge.window_duration.as_str(),
                "Unable to enqueue budget window spend update"
            );
            None
        }
    }
}

// one budget_limits entry as the facts its counter and row are built from; entry without a string duration names no counter
pub(super) fn window_charge(
    scope: &WindowScope,
    value: &JsonData,
    charge: &ScopeCharge,
    now: DateTime<Utc>,
    durations: &dyn DurationParser,
) -> Option<WindowCharge> {
    let window = coerce_window(value);
    let Some(JsonData::String(window_duration)) = window.as_ref().get("budget_duration") else {
        tracing::warn!(
            entity_type = scope.entity_type.wire_value(),
            "Skipping budget window with invalid duration"
        );
        return None;
    };
    Some(WindowCharge {
        counter_key: format!("{}:window:{window_duration}", scope.counter_key),
        entity_type: scope.entity_type.clone(),
        entity_id: scope.entity_id.clone(),
        window_duration: window_duration.clone(),
        window_start: budget_window_start(&window, now, durations),
        reset_at: has_reset_at(&window),
        cost: charge.cost,
        request_started_at: charge.request_started_at,
    })
}

// a key or team window counts as one row only when reset_at names an instant; a null or empty string slides
fn has_reset_at(window: &BudgetWindow) -> bool {
    match window.as_ref().get("reset_at") {
        Some(JsonData::Null) | None => false,
        Some(JsonData::String(text)) => !text.is_empty(),
        Some(
            JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) | JsonData::Object(_),
        ) => true,
    }
}
