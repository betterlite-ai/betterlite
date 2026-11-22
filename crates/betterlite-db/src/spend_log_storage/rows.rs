use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::Notify;

use crate::redis_spend::log_row::SpendLogRow;
use crate::spend_batches::{spend_log_queue_within_budget, spend_log_row_bytes};

pub mod parking;
pub mod writer;

mod tests;

// MAX_LOGS_PER_INTERVAL: one pass takes at most this many rows off the queue
pub const MAX_LOGS_PER_INTERVAL: usize = 10_000;

// Where a replayed batch joins the queue: a batch the database refused goes back to the head, a new request to the tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueuePosition {
    Head,
    Tail,
}

#[derive(Debug, Default)]
struct QueueState {
    rows: Vec<SpendLogRow>,
    bytes: usize,
}

// enqueue_spend_logs and dequeue_spend_logs: the rows and their byte total move together under one lock.
pub struct SpendLogQueue {
    state: Mutex<QueueState>,
    max_bytes: usize,
    // spend_log_flush_requested: None until a monitor installs a signal, so a request made before it retains nothing.
    flush_requested: Mutex<Option<Arc<Notify>>>,
}

impl SpendLogQueue {
    #[must_use]
    pub fn new(max_bytes: usize) -> Self {
        Self {
            state: Mutex::new(QueueState::default()),
            max_bytes,
            flush_requested: Mutex::new(None),
        }
    }

    // enqueue_spend_logs: the byte total is trimmed to the budget oldest first, and the newest row stays however large it is
    pub fn enqueue(&self, rows: &[SpendLogRow], position: QueuePosition) {
        if rows.is_empty() {
            return;
        }
        let added: usize = rows.iter().map(spend_log_row_bytes).sum();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut queued = Vec::with_capacity(state.rows.len().saturating_add(rows.len()));
        match position {
            QueuePosition::Head => {
                queued.extend_from_slice(rows);
                queued.extend_from_slice(&state.rows);
            }
            QueuePosition::Tail => {
                queued.extend_from_slice(&state.rows);
                queued.extend_from_slice(rows);
            }
        }
        let budget = state.bytes.saturating_add(added);
        let (kept, kept_bytes) = {
            let trimmed = spend_log_queue_within_budget(&queued, budget, self.max_bytes);
            (trimmed.rows.len(), trimmed.queued_bytes)
        };
        let dropped = queued.len().saturating_sub(kept);
        let keep_from = queued.len().saturating_sub(kept);
        let _removed = queued.drain(..keep_from);
        state.rows = queued;
        state.bytes = kept_bytes;
        if let Some(dropped_rows) = NonZeroUsize::new(dropped) {
            let max_bytes = self.max_bytes;
            tracing::error!(
                "Spend tracking - spend log queue is at its {max_bytes} byte budget; dropped the {dropped_rows} oldest spend logs"
            );
        }
    }

    // dequeue_spend_logs: pops the oldest rows and subtracts exactly what they cost
    #[must_use]
    pub fn dequeue(&self, limit: usize) -> Vec<SpendLogRow> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let taken = state.rows.len().min(limit);
        let popped: Vec<SpendLogRow> = state.rows.drain(..taken).collect();
        let freed: usize = popped.iter().map(spend_log_row_bytes).sum();
        state.bytes = state.bytes.saturating_sub(freed);
        popped
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .rows
            .len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn queued_bytes(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .bytes
    }

    // request_spend_log_flush: repeated requests coalesce into the monitor's next pass, and without a monitor there is nowhere to keep one
    pub fn request_flush(&self) {
        let requested = self
            .flush_requested
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(signal) = requested.as_deref() {
            signal.notify_one();
        }
    }

    // the monitor installs its signal when it starts and clears it on shutdown
    pub fn set_flush_signal(&self, signal: Option<Arc<Notify>>) {
        let mut requested = self
            .flush_requested
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *requested = signal;
    }
}
