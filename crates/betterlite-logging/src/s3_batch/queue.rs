use crate::s3_batch::Element;
use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
#[derive(Debug)]
pub struct BatchQueue {
    pub state: Mutex<QueueState>,
    pub max_queue_size: usize,
}
#[derive(Debug, Default)]
pub struct QueueState {
    pub entries: VecDeque<Element>,
    pub requeued_count: usize,
}
#[derive(Debug, Default)]
pub struct Snapshot {
    pub entries: Vec<Element>,
    pub requeued_count: usize,
}
impl BatchQueue {
    pub fn enqueue(&self, element: Element) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entries
            .push_back(element);
    }
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        Snapshot {
            entries: state.entries.iter().cloned().collect(),
            requeued_count: state.requeued_count,
        }
    }
    pub fn finish_success(&self, batch_len: usize) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let settled = batch_len.min(state.entries.len());
        state.entries.drain(..settled);
        state.requeued_count = 0;
    }
    pub fn restore_after_failure(&self, batch_len: usize, requeued: Vec<Element>) -> usize {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
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
    pub fn trim_overflow(&self) -> usize {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let restored = self.trim_locked(&mut state);
        drop(state);
        restored
    }
    fn trim_locked(&self, state: &mut QueueState) -> usize {
        let dropped = state.entries.len().saturating_sub(self.max_queue_size);
        state.entries.drain(..dropped);
        state.requeued_count = state.requeued_count.saturating_sub(dropped);
        dropped
    }
}
mod tests;
