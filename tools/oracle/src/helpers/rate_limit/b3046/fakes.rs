use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_core::api::batch_enqueued_refund::BatchEnqueuedRefundError;
use betterlite_core::api::batch_enqueued_refund::record::ReservationRecord;
use betterlite_core::api::batch_enqueued_reserve::BatchEnqueuedTokenReservation;
use betterlite_rate_limit::concurrency_gauges::cache::{GaugeCache, GaugeCacheValue};
use betterlite_rate_limit::limiter_records::CacheCounterValue;
use betterlite_rate_limit::post_call_settlement::batch_enqueued::BatchEnqueuedTokenStore;
use betterlite_rate_limit::rate_limit_window_store::{CounterCache, CounterValues, RedisCacheKind};
use betterlite_rate_limit::token_adjustments::store::UsageIncrements;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use serde_json::{Value, json};

pub(crate) const WINDOW: i64 = 60;

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn operation_json(operation: &IncrementOperation) -> Value {
    json!({
        "key": &operation.key,
        "increment_value": operation.increment_value,
        "ttl": operation.ttl,
    })
}

fn gauge_json(value: &GaugeCacheValue) -> Value {
    match value {
        GaugeCacheValue::Registry(registry) => json!(registry),
        GaugeCacheValue::Counter(CacheCounterValue::Int(number)) => json!(number),
        GaugeCacheValue::Counter(CacheCounterValue::Float(number)) => json!(number),
        GaugeCacheValue::Counter(CacheCounterValue::Text(text)) => json!(text),
        GaugeCacheValue::Counter(CacheCounterValue::Bytes(bytes)) => json!(bytes),
    }
}

#[derive(Default)]
pub(crate) struct FakeUsage {
    increments: Mutex<Vec<IncrementOperation>>,
    pipelines: Mutex<Vec<Vec<IncrementOperation>>>,
    local_sets: Mutex<Vec<(String, f64, Option<i64>)>>,
}

impl FakeUsage {
    pub(crate) fn increments(&self) -> Vec<Value> {
        lock(&self.increments).iter().map(operation_json).collect()
    }

    pub(crate) fn pipelines(&self) -> Vec<Value> {
        lock(&self.pipelines)
            .iter()
            .map(|operations| operations.iter().map(operation_json).collect())
            .collect()
    }

    pub(crate) fn local_sets(&self) -> Vec<Value> {
        lock(&self.local_sets)
            .iter()
            .map(|(key, value, ttl)| json!({"key": key, "value": value, "ttl": ttl}))
            .collect()
    }
}

#[async_trait::async_trait]
impl UsageIncrements for FakeUsage {
    async fn increment(&self, operation: &IncrementOperation) {
        lock(&self.increments).push(operation.clone());
    }

    async fn increment_pipeline(&self, operations: &[IncrementOperation]) {
        lock(&self.pipelines).push(operations.to_vec());
    }

    async fn set_local_counter(&self, key: &str, value: f64, ttl: Option<i64>) {
        lock(&self.local_sets).push((key.to_owned(), value, ttl));
    }
}

pub(crate) struct RecordingCounters {
    values: Mutex<BTreeMap<String, CacheCounterValue>>,
}

impl RecordingCounters {
    pub(crate) fn seeded(seed: &BTreeMap<String, i64>) -> Self {
        let values = seed
            .iter()
            .map(|(key, value)| (key.clone(), CacheCounterValue::Int(*value)))
            .collect();
        Self {
            values: Mutex::new(values),
        }
    }
}

#[async_trait::async_trait]
impl CounterCache for RecordingCounters {
    async fn get_cache(&self, key: &str, _local_only: bool) -> Option<CacheCounterValue> {
        lock(&self.values).get(key).cloned()
    }

    async fn set_cache(&self, key: &str, value: CacheCounterValue, _ttl: i64, _local_only: bool) {
        lock(&self.values).insert(key.to_owned(), value);
    }

    async fn batch_get_cache(&self, keys: &[String], _local_only: bool) -> Option<CounterValues> {
        let values = lock(&self.values);
        Some(keys.iter().map(|key| values.get(key).cloned()).collect())
    }

    fn redis_cache_kind(&self) -> Option<RedisCacheKind> {
        None
    }
}

pub(crate) struct RecordingGauges {
    values: Mutex<BTreeMap<String, GaugeCacheValue>>,
    gauge_keys: BTreeSet<String>,
    reads: Mutex<Vec<Value>>,
    writes: Mutex<Vec<Value>>,
}

impl RecordingGauges {
    pub(crate) fn seeded(seed: &BTreeMap<String, i64>, gauge_keys: &[String]) -> Self {
        let values = seed
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    GaugeCacheValue::Counter(CacheCounterValue::Int(*value)),
                )
            })
            .collect();
        Self {
            values: Mutex::new(values),
            gauge_keys: gauge_keys.iter().cloned().collect(),
            reads: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn reads(&self) -> Vec<Value> {
        lock(&self.reads).clone()
    }

    pub(crate) fn writes(&self) -> Vec<Value> {
        lock(&self.writes).clone()
    }
}

#[async_trait::async_trait]
impl GaugeCache for RecordingGauges {
    async fn get(&self, key: &str, _local_only: bool) -> Option<GaugeCacheValue> {
        if self.gauge_keys.contains(key) {
            lock(&self.reads).push(json!(key));
        }
        lock(&self.values).get(key).cloned()
    }

    async fn set(&self, key: &str, value: GaugeCacheValue, ttl: Duration, _local_only: bool) {
        lock(&self.writes)
            .push(json!({"key": key, "value": gauge_json(&value), "ttl": ttl.as_secs()}));
        lock(&self.values).insert(key.to_owned(), value);
    }

    async fn batch_get(
        &self,
        keys: &[String],
        _local_only: bool,
    ) -> Option<Vec<Option<GaugeCacheValue>>> {
        let values = lock(&self.values);
        Some(keys.iter().map(|key| values.get(key).cloned()).collect())
    }
}

pub(crate) struct FakeBatchStore {
    calls: Mutex<Vec<Value>>,
    popped: Option<ReservationRecord>,
}

impl FakeBatchStore {
    pub(crate) fn new(popped: Option<ReservationRecord>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            popped,
        }
    }

    fn record(&self, call: Value) {
        lock(&self.calls).push(call);
    }

    pub(crate) fn calls(&self) -> Vec<Value> {
        lock(&self.calls).clone()
    }
}

#[async_trait::async_trait]
impl BatchEnqueuedTokenStore for FakeBatchStore {
    async fn save_reservation(
        &self,
        batch_id: &str,
        reservation: &BatchEnqueuedTokenReservation,
    ) -> Result<(), BatchEnqueuedRefundError> {
        self.record(json!({"call": "save", "batch_id": batch_id, "tokens": reservation.tokens}));
        Ok(())
    }

    async fn pop_reservation(
        &self,
        batch_id: &str,
    ) -> Result<Option<ReservationRecord>, BatchEnqueuedRefundError> {
        self.record(json!({"call": "pop", "batch_id": batch_id}));
        Ok(self.popped.clone())
    }

    async fn refund(
        &self,
        reservation: &ReservationRecord,
    ) -> Result<(), BatchEnqueuedRefundError> {
        self.record(json!({"call": "refund", "tokens": reservation.tokens}));
        Ok(())
    }
}
