use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard};

use crate::s3_batch::Element;

#[derive(Debug)]
pub struct BatchQueue {
    state: Mutex<QueueState>,
    max_queue_size: usize,
}

#[derive(Debug, Default)]
struct QueueState {
    entries: VecDeque<Element>,
    requeued_count: usize,
}

#[derive(Debug, Default)]
pub struct Snapshot {
    pub entries: Vec<Element>,
    pub requeued_count: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Restored {
    pub dropped_oldest: usize,
}

fn lock(state: &Mutex<QueueState>) -> MutexGuard<'_, QueueState> {
    match state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl BatchQueue {
    #[must_use]
    pub fn with_capacity(max_queue_size: usize) -> Self {
        Self {
            state: Mutex::new(QueueState::default()),
            max_queue_size,
        }
    }

    pub fn enqueue(&self, element: Element) -> usize {
        let mut state = lock(&self.state);
        state.entries.push_back(element);
        state.entries.len()
    }

    // the flush works on a snapshot so events appended meanwhile survive a successful send
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        let state = lock(&self.state);
        Snapshot {
            entries: state.entries.iter().cloned().collect(),
            requeued_count: state.requeued_count,
        }
    }

    pub fn finish_success(&self, batch_len: usize) {
        let mut state = lock(&self.state);
        let settled = batch_len.min(state.entries.len());
        state.entries.drain(..settled);
        state.requeued_count = 0;
    }

    // what the first flush sent is gone from the queue, so its requeue lands before the arrivals
    pub fn restore_after_failure(&self, batch_len: usize, requeued: Vec<Element>) -> Restored {
        let mut state = lock(&self.state);
        let arrivals: Vec<Element> = state.entries.iter().skip(batch_len).cloned().collect();
        let requeued_len = requeued.len();
        state.entries = requeued
            .into_iter()
            .chain(arrivals)
            .collect::<VecDeque<Element>>();
        state.requeued_count = requeued_len;
        let restored = self.trim_locked(&mut state);
        drop(state);
        restored
    }

    // the inherited flush loop trims a queue that grew while its send failed with no requeue plan
    pub fn trim_overflow(&self) -> Restored {
        let mut state = lock(&self.state);
        let restored = self.trim_locked(&mut state);
        drop(state);
        restored
    }

    fn trim_locked(&self, state: &mut QueueState) -> Restored {
        let dropped = state.entries.len().saturating_sub(self.max_queue_size);
        state.entries.drain(..dropped);
        let requeued_count = state.requeued_count.saturating_sub(dropped);
        state.requeued_count = requeued_count;
        Restored {
            dropped_oldest: dropped,
        }
    }
}

mod tests;
