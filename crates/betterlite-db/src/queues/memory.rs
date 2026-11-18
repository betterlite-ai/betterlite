use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::mpsc;

mod tests;

// BaseUpdateQueue of the source: one bounded queue whose producer waits for capacity, a capped FIFO drain, and an enqueue observer.

#[derive(Debug, thiserror::Error)]
pub enum MemoryQueueError {
    #[error("the in-memory spend queue needs a capacity of at least one record")]
    ZeroCapacity,
    #[error("the in-memory spend queue is closed")]
    Closed,
}

// _emit_new_item_added_to_queue_event: the base queue does nothing, and each later implementation reports the queue size as a gauge.
pub trait EnqueueObserver: Send + Sync {
    fn item_added(&self, queue_size: usize);
}

#[derive(Debug, Default)]
pub struct NoEnqueueObserver;

impl EnqueueObserver for NoEnqueueObserver {
    fn item_added(&self, _queue_size: usize) {}
}

pub struct MemoryUpdateQueue<T> {
    sender: mpsc::Sender<T>,
    receiver: Mutex<mpsc::Receiver<T>>,
    aggregation_threshold: i64,
    flush_count: i64,
    observer: Arc<dyn EnqueueObserver>,
}

impl<T: Send> MemoryUpdateQueue<T> {
    pub fn new(
        capacity: usize,
        aggregation_threshold: i64,
        flush_count: i64,
        observer: Arc<dyn EnqueueObserver>,
    ) -> Result<Self, MemoryQueueError> {
        if capacity == 0 {
            return Err(MemoryQueueError::ZeroCapacity);
        }
        let (sender, receiver) = mpsc::channel(capacity);
        let queue = Self {
            sender,
            receiver: Mutex::new(receiver),
            aggregation_threshold,
            flush_count,
            observer,
        };
        let stored_threshold = queue.aggregation_threshold;
        if i64::try_from(capacity).is_ok_and(|queue_capacity| stored_threshold >= queue_capacity) {
            tracing::warn!(
                aggregation_threshold = stored_threshold,
                capacity,
                "Misconfigured queue thresholds: MAX_SIZE_IN_MEMORY_QUEUE ({stored_threshold}) >= LITELLM_ASYNCIO_QUEUE_MAXSIZE ({capacity}). The spend aggregation check will never trigger because the asyncio.Queue blocks at {capacity} items. Set MAX_SIZE_IN_MEMORY_QUEUE to a value less than LITELLM_ASYNCIO_QUEUE_MAXSIZE (recommended: 80% of it)."
            );
        }
        Ok(queue)
    }

    // A cancelled add never admitted its record: reserve hands the permit over only once the queue has room.
    pub async fn add_update(&self, update: T) -> Result<(), MemoryQueueError> {
        // The source logs the whole update here; betterlite's log line keeps spend bodies out of every log.
        tracing::debug!("Adding update to the in-memory spend queue");
        let admission = self
            .sender
            .reserve()
            .await
            .map_err(|_source| MemoryQueueError::Closed)?;
        admission.send(update);
        // qsize() in the source counts the records waiting in the queue, not the admissions other producers hold.
        let queue_size = self
            .receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        self.observer.item_added(queue_size);
        Ok(())
    }

    #[must_use]
    pub fn drain_updates(&self) -> Vec<T> {
        let mut updates = Vec::new();
        {
            let mut receiver = self.receiver.lock().unwrap_or_else(PoisonError::into_inner);
            while !receiver.is_empty() {
                // Circuit breaker: the source stops a pass at MAX_IN_MEMORY_QUEUE_FLUSH_COUNT, and a nonpositive value therefore drains nothing.
                if i64::try_from(updates.len()).unwrap_or(i64::MAX) >= self.flush_count {
                    tracing::debug!("Max in memory queue flush count reached, stopping flush");
                    break;
                }
                let Ok(update) = receiver.try_recv() else {
                    break;
                };
                updates.push(update);
            }
        }
        updates
    }

    pub fn close(&self) {
        self.receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .close();
    }
}
