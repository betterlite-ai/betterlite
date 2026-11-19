use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use tokio::sync::Notify;

use super::{WindowQueueError, WindowSpendTransaction, aggregate_window_spend};

// WindowSpendUpdateQueue: asyncio.Queue(capacity) plus the compaction pass, with the drain and reinsert of one compaction under the queue lock so a compaction never waits on the queue it is refilling.
pub struct WindowSpendUpdateQueue {
    state: Mutex<QueueState>,
    room: Notify,
    capacity: usize,
    threshold: i64,
    flush_limit: usize,
}

struct QueueState {
    batches: VecDeque<Vec<WindowSpendTransaction>>,
}

impl WindowSpendUpdateQueue {
    pub fn new(
        queue_capacity: i64,
        aggregation_threshold: i64,
        flush_count: i64,
    ) -> Result<Self, WindowQueueError> {
        let capacity = match usize::try_from(queue_capacity) {
            Ok(capacity) if capacity > 0 => capacity,
            Ok(_empty) | Err(_range) => return Err(WindowQueueError::ZeroCapacity),
        };
        Ok(Self {
            state: Mutex::new(QueueState {
                batches: VecDeque::new(),
            }),
            room: Notify::new(),
            capacity,
            threshold: aggregation_threshold,
            flush_limit: usize::try_from(flush_count).unwrap_or_default(),
        })
    }

    // add_update then aggregate_queue_updates: the producer waits for a slot, and the queue compacts once it reaches the threshold.
    pub async fn add_update(&self, record: WindowSpendTransaction) -> Result<(), WindowQueueError> {
        loop {
            let freed = self.room.notified();
            tokio::pin!(freed);
            // register before the check, so a flush that frees a slot right after cannot miss this waiter
            freed.as_mut().enable();
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.batches.len() < self.capacity {
                state.batches.push_back(vec![record]);
                drop(state);
                return self.compact_if_due();
            }
            drop(state);
            freed.await;
        }
    }

    // flush_and_get_aggregated_window_spend_transactions: one drained pass of at most MAX_IN_MEMORY_QUEUE_FLUSH_COUNT batches, aggregated per window. A failed aggregation puts the batches back, where the source has already dropped them.
    pub fn flush(&self) -> Result<Vec<WindowSpendTransaction>, WindowQueueError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let take = state.batches.len().min(self.flush_limit);
        let drained: Vec<Vec<WindowSpendTransaction>> = state.batches.drain(..take).collect();
        let aggregated = match aggregate_window_spend(&drained) {
            Ok(aggregated) => aggregated,
            Err(error) => {
                for batch in drained.into_iter().rev() {
                    state.batches.push_front(batch);
                }
                return Err(error);
            }
        };
        drop(state);
        if !drained.is_empty() {
            tracing::info!(
                batches = take,
                "Spend tracking - flushed budget window spend update batches from in-memory queue"
            );
            self.room.notify_waiters();
        }
        Ok(aggregated)
    }

    // aggregate_queue_updates: everything queued up to the flush count becomes one batch of merged windows, and the batches it replaced free their slots.
    fn compact_if_due(&self) -> Result<(), WindowQueueError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let queued = i64::try_from(state.batches.len()).unwrap_or(i64::MAX);
        if queued < self.threshold {
            return Ok(());
        }
        tracing::warn!(
            "Budget window spend update queue is full. Aggregating all entries in queue to concatenate entries."
        );
        let take = state.batches.len().min(self.flush_limit);
        let drained: Vec<Vec<WindowSpendTransaction>> = state.batches.drain(..take).collect();
        let aggregated = match aggregate_window_spend(&drained) {
            Ok(aggregated) => aggregated,
            Err(error) => {
                // the source loses the drained increments here; betterlite puts them back before reporting the failure
                for batch in drained.into_iter().rev() {
                    state.batches.push_front(batch);
                }
                return Err(error);
            }
        };
        if !aggregated.is_empty() {
            state.batches.push_back(aggregated);
        }
        drop(state);
        // Notify keeps no count, so waking a waiter that finds no room only re-parks it
        self.room.notify_waiters();
        Ok(())
    }
}
