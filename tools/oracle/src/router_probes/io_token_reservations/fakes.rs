use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_router::io_token_reservations::counter::{CountObservation, IoCounter};
use betterlite_router::io_token_reservations::estimate::{InputTokenCounter, IoInputField};

#[derive(Default)]
pub(crate) struct ProbeCounter {
    values: Mutex<BTreeMap<String, i64>>,
}

impl ProbeCounter {
    pub(crate) fn seed(&self, key: &str, value: i64) {
        if let Ok(mut values) = self.values.lock() {
            values.insert(key.to_owned(), value);
        }
    }

    pub(crate) fn value(&self, key: &str) -> i64 {
        self.values
            .lock()
            .map(|values| values.get(key).copied().unwrap_or_default())
            .unwrap_or_default()
    }
}

#[async_trait]
impl IoCounter for ProbeCounter {
    async fn increment_counter(
        &self,
        key: &str,
        value: i64,
        _ttl_seconds: u64,
    ) -> CountObservation {
        let Ok(mut values) = self.values.lock() else {
            return CountObservation::Unavailable;
        };
        let Some(current) = values
            .get(key)
            .copied()
            .unwrap_or_default()
            .checked_add(value)
        else {
            return CountObservation::Overflow;
        };
        values.insert(key.to_owned(), current);
        drop(values);
        CountObservation::Counted(current)
    }
}

pub(crate) struct ProbeEstimator {
    pub(crate) estimate: i64,
}

#[async_trait]
impl InputTokenCounter for ProbeEstimator {
    async fn count_field(
        &self,
        _model: &str,
        _field: IoInputField,
        _value: &Json,
    ) -> Option<usize> {
        usize::try_from(self.estimate).ok()
    }
}
