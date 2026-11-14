use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::api::batch_enqueued_refund::BatchEnqueuedRefundError;
use betterlite_core::api::batch_enqueued_refund::store::EnqueuedLocalStore;
use betterlite_core::api::batch_enqueued_reserve::store::LocalTokenCounters;
use serde_json::{Value, json};

pub(super) struct RecordingStore {
    pub(super) counters: LocalTokenCounters,
    pub(super) events: Arc<Mutex<Vec<Value>>>,
}

impl RecordingStore {
    fn push_event(&self, event: Value) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    fn finish_local_set(
        &self,
        key: &str,
        value: &Value,
        ttl_seconds: i64,
        outcome: Result<(), BatchEnqueuedRefundError>,
    ) -> Result<(), BatchEnqueuedRefundError> {
        if outcome.is_ok() {
            let event =
                json!({"event": "local_set", "key": key, "value": value, "args": [ttl_seconds]});
            self.push_event(event);
        }
        outcome
    }
}

impl EnqueuedLocalStore for RecordingStore {
    fn counter(&mut self, counter_key: &str) -> Result<i64, BatchEnqueuedRefundError> {
        EnqueuedLocalStore::counter(&mut self.counters, counter_key)
    }

    fn set_counter(
        &mut self,
        counter_key: &str,
        value: i64,
        ttl_seconds: i64,
    ) -> Result<(), BatchEnqueuedRefundError> {
        let outcome =
            EnqueuedLocalStore::set_counter(&mut self.counters, counter_key, value, ttl_seconds);
        self.finish_local_set(counter_key, &json!(value), ttl_seconds, outcome)
    }

    fn delete(&mut self, key: &str) {
        self.push_event(json!({"event": "local_delete", "key": key}));
        EnqueuedLocalStore::delete(&mut self.counters, key);
    }

    fn record(&mut self, record_key: &str) -> Result<Option<Vec<u8>>, BatchEnqueuedRefundError> {
        EnqueuedLocalStore::record(&mut self.counters, record_key)
    }

    fn set_record(
        &mut self,
        record_key: &str,
        record: &str,
        ttl_seconds: i64,
    ) -> Result<(), BatchEnqueuedRefundError> {
        let outcome =
            EnqueuedLocalStore::set_record(&mut self.counters, record_key, record, ttl_seconds);
        let stored: Value = serde_json::from_str(record).unwrap_or(Value::Null);
        self.finish_local_set(record_key, &stored, ttl_seconds, outcome)
    }
}
