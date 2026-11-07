use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use super::record::Payload;

pub struct PayloadQueue {
    entries: Mutex<VecDeque<Payload>>,
    max_queue_size: usize,
}

impl PayloadQueue {
    #[must_use]
    pub const fn new(max_queue_size: usize) -> Self {
        Self {
            entries: Mutex::new(VecDeque::new()),
            max_queue_size,
        }
    }

    pub fn enqueue(&self, payload: Payload) -> usize {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.push_back(payload);
        let overflow = entries.len().saturating_sub(self.max_queue_size);
        entries.drain(..overflow);
        let depth = entries.len();
        drop(entries);
        if overflow > 0 {
            tracing::warn!(
                max_queue_size = self.max_queue_size,
                dropped = overflow,
                "OpikLogger log queue exceeded max_queue_size; dropped the oldest events"
            );
        }
        depth
    }

    pub fn snapshot(&self) -> Vec<Payload> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    pub fn clear(&self) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
}
