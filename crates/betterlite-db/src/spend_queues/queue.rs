use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use tokio::sync::Notify;

use super::SpendQueueError;
use crate::queues::memory::EnqueueObserver;

// BaseUpdateQueue.update_queue of the source, with one difference the source's asyncio.Queue cannot offer: the drain and reinsert of a compaction run under this lock, so a compaction never waits on a queue it is refilling.
pub(super) struct BoundedUpdateQueue<T> {
    items: Mutex<VecDeque<T>>,
    space: Notify,
    // A settling interrupted commit that needs room rings this bell, so a waiting flush drains the queue instead of deadlocking against the settlement it is waiting for.
    pub(super) pressure: Notify,
    capacity: usize,
    aggregation_threshold: i64,
    flush_count: i64,
}

impl<T: Send> BoundedUpdateQueue<T> {
    pub(super) fn new(
        queue_capacity: i64,
        aggregation_threshold: i64,
        flush_count: i64,
    ) -> Result<Self, SpendQueueError> {
        let Ok(capacity) = usize::try_from(queue_capacity) else {
            return Err(SpendQueueError::ZeroCapacity);
        };
        if capacity == 0 {
            return Err(SpendQueueError::ZeroCapacity);
        }
        let queue = Self {
            items: Mutex::new(VecDeque::new()),
            space: Notify::new(),
            pressure: Notify::new(),
            capacity,
            aggregation_threshold,
            flush_count,
        };
        let stored_threshold = queue.aggregation_threshold;
        if i64::try_from(capacity).is_ok_and(|stored_capacity| stored_threshold >= stored_capacity)
        {
            tracing::warn!(
                aggregation_threshold = stored_threshold,
                capacity,
                "Misconfigured queue thresholds: MAX_SIZE_IN_MEMORY_QUEUE ({stored_threshold}) >= LITELLM_ASYNCIO_QUEUE_MAXSIZE ({capacity}). The spend aggregation check will never trigger because the queue blocks at {capacity} items. Set MAX_SIZE_IN_MEMORY_QUEUE to a value less than LITELLM_ASYNCIO_QUEUE_MAXSIZE (recommended: 80% of it)."
            );
        }
        Ok(queue)
    }

    // asyncio.Queue.put: a producer waits for room, and a cancelled producer still owns its record.
    pub(super) async fn add(&self, update: T, observer: &dyn EnqueueObserver) {
        loop {
            let space = self.space.notified();
            tokio::pin!(space);
            space.as_mut().enable();
            {
                let mut items = self.items.lock().unwrap_or_else(PoisonError::into_inner);
                if items.len() < self.capacity {
                    items.push_back(update);
                    let queue_size = items.len();
                    drop(items);
                    observer.item_added(queue_size);
                    return;
                }
            }
            // notify_one keeps a permit, so a flush that has not started waiting yet still sees the pressure.
            self.pressure.notify_one();
            space.await;
        }
    }

    // flush_all_updates_from_in_memory_queue: at most MAX_IN_MEMORY_QUEUE_FLUSH_COUNT records, oldest first.
    pub(super) fn drain(&self) -> Vec<T> {
        let mut items = self.items.lock().unwrap_or_else(PoisonError::into_inner);
        let drained = take_up_to(&mut items, self.flush_count);
        drop(items);
        self.space.notify_waiters();
        drained
    }

    // aggregate_queue_updates: one pass over the records queued right now, with the aggregated records put back in their place.
    pub(super) fn compact_when_full(
        &self,
        aggregate: fn(&[T]) -> Result<Vec<T>, SpendQueueError>,
    ) -> Result<(), SpendQueueError> {
        let mut items = self.items.lock().unwrap_or_else(PoisonError::into_inner);
        // The source compacts once qsize() reaches MAX_SIZE_IN_MEMORY_QUEUE, so a threshold of zero compacts even an empty queue.
        if i64::try_from(items.len()).unwrap_or(i64::MAX) < self.aggregation_threshold {
            return Ok(());
        }
        tracing::warn!(
            "Spend update queue is full. Aggregating all entries in queue to concatenate entries."
        );
        let drained = take_up_to(&mut items, self.flush_count);
        match aggregate(&drained) {
            Ok(aggregated) => {
                items.extend(aggregated);
                drop(items);
                self.space.notify_waiters();
                Ok(())
            }
            // The source raises after its drain and loses the drained records; betterlite puts them back before reporting the failure.
            Err(error) => {
                items.extend(drained);
                Err(error)
            }
        }
    }
}

fn take_up_to<T>(items: &mut VecDeque<T>, flush_count: i64) -> Vec<T> {
    let limit = usize::try_from(flush_count).unwrap_or_default();
    let take = items.len().min(limit);
    items.drain(..take).collect()
}
