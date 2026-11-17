use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use crate::fields::payload::StandardLoggingPayload;

#[derive(Debug, Default)]
pub struct PendingQueue {
    records: Mutex<VecDeque<StandardLoggingPayload>>,
}

impl PendingQueue {
    pub fn push(&self, record: StandardLoggingPayload) {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(record);
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }

    #[must_use]
    pub fn snapshot(&self) -> Vec<StandardLoggingPayload> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    pub fn remove_prefix(&self, count: usize) {
        let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        let removed = count.min(records.len());
        records.drain(..removed);
    }

    #[must_use]
    pub fn drop_overflow(&self, max_records: usize) -> usize {
        let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        let dropped = records.len().saturating_sub(max_records);
        records.drain(..dropped);
        dropped
    }
}
