use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use opentelemetry_sdk::trace::SpanData;
use serde_json::{Value, json};

// What the exporter did during one call: the spans each body carried, the requests sent and the waits between
#[derive(Debug, Default)]
pub(crate) struct Journal {
    encodes: Mutex<Vec<Vec<SpanData>>>,
    requests: Mutex<Vec<Value>>,
    sleeps: Mutex<Vec<Duration>>,
}

impl Journal {
    pub(crate) fn record_encode(&self, spans: &[SpanData]) -> u64 {
        let mut encodes = self.encodes.lock().unwrap_or_else(PoisonError::into_inner);
        encodes.push(spans.to_vec());
        u64::try_from(encodes.len().saturating_sub(1)).unwrap_or(u64::MAX)
    }
    pub(crate) fn encoded(&self, index: u64) -> Option<Vec<SpanData>> {
        let position = usize::try_from(index).ok()?;
        self.encodes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(position)
            .cloned()
    }
    pub(crate) fn record_request(&self, request: Value) {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
    }

    pub(crate) fn record_sleep(&self, delay: Duration) {
        self.sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(delay);
    }

    pub(crate) fn answer(&self, delivered: bool) -> Value {
        let requests = self
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let sleeps: Vec<u64> = self
            .sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(Duration::as_secs)
            .collect();
        json!({
            "result": if delivered { "success" } else { "failure" },
            "requests": requests,
            "sleeps": sleeps,
        })
    }
}
