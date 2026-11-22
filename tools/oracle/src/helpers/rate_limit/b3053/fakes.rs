use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_core::api::batch_enqueued_refund::BatchEnqueuedRefundError;
use betterlite_core::api::batch_enqueued_refund::record::ReservationRecord;
use betterlite_core::api::batch_enqueued_reserve::BatchEnqueuedTokenReservation;
use betterlite_rate_limit::concurrency_gauges::cache::{GaugeCache, GaugeCacheValue};
use betterlite_rate_limit::limiter_records::{CacheCounterValue, LimiterClock};
use betterlite_rate_limit::post_call_settlement::batch_enqueued::BatchEnqueuedTokenStore;
use betterlite_rate_limit::rate_limit_window_store::{CounterCache, CounterValues, RedisCacheKind};
use betterlite_rate_limit::token_adjustments::store::UsageIncrements;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

// the slot release reads each gauge and finds no entry, so the fake answers None and never writes
pub(crate) struct SilentGauges;

#[async_trait::async_trait]
impl GaugeCache for SilentGauges {
    async fn get(&self, _key: &str, _local_only: bool) -> Option<GaugeCacheValue> {
        None
    }

    async fn set(&self, _key: &str, _value: GaugeCacheValue, _ttl: Duration, _local_only: bool) {}

    async fn batch_get(
        &self,
        keys: &[String],
        _local_only: bool,
    ) -> Option<Vec<Option<GaugeCacheValue>>> {
        Some(keys.iter().map(|_key| None).collect())
    }
}

// the hook never reads a window counter; the fake only has to let the store be built
pub(crate) struct SilentCounters;

#[async_trait::async_trait]
impl CounterCache for SilentCounters {
    async fn get_cache(&self, _key: &str, _local_only: bool) -> Option<CacheCounterValue> {
        None
    }

    async fn set_cache(&self, _key: &str, _value: CacheCounterValue, _ttl: i64, _local_only: bool) {
    }

    async fn batch_get_cache(&self, keys: &[String], _local_only: bool) -> Option<CounterValues> {
        Some(keys.iter().map(|_key| None).collect())
    }

    fn redis_cache_kind(&self) -> Option<RedisCacheKind> {
        None
    }
}

pub(crate) struct EpochClock;

impl LimiterClock for EpochClock {
    fn current_time(&self) -> SystemTime {
        UNIX_EPOCH
    }
}

// neither hook settles a batch reservation, so every batch call is a no-op
pub(crate) struct SilentStore;

#[async_trait::async_trait]
impl BatchEnqueuedTokenStore for SilentStore {
    async fn save_reservation(
        &self,
        _batch_id: &str,
        _reservation: &BatchEnqueuedTokenReservation,
    ) -> Result<(), BatchEnqueuedRefundError> {
        Ok(())
    }

    async fn pop_reservation(
        &self,
        _batch_id: &str,
    ) -> Result<Option<ReservationRecord>, BatchEnqueuedRefundError> {
        Ok(None)
    }

    async fn refund(
        &self,
        _reservation: &ReservationRecord,
    ) -> Result<(), BatchEnqueuedRefundError> {
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct RecordedPipelines {
    pub(crate) lines: Mutex<Vec<Vec<IncrementOperation>>>,
}

#[async_trait::async_trait]
impl UsageIncrements for RecordedPipelines {
    async fn increment(&self, operation: &IncrementOperation) {
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(vec![operation.clone()]);
    }

    async fn increment_pipeline(&self, operations: &[IncrementOperation]) {
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(operations.to_vec());
    }

    async fn set_local_counter(&self, _key: &str, _value: f64, _ttl: Option<i64>) {}
}

#[derive(Clone, Default)]
pub(crate) struct PriorityLogs(pub(crate) Arc<Mutex<Vec<String>>>);

impl<S: Subscriber> Layer<S> for PriorityLogs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut visitor = PriorityField::default();
        event.record(&mut visitor);
        if let Some(priority) = visitor.0 {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(priority);
        }
    }
}

#[derive(Default)]
struct PriorityField(Option<String>);

impl Visit for PriorityField {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "priority" {
            self.0 = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "priority" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}
