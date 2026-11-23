use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::baseline::records::BaselineAccountingRecord;

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("the baseline observation queue needs a capacity of at least one record")]
    ZeroCapacity,
    #[error("the baseline observation queue capacity exceeds what a semaphore can count")]
    CapacityTooLarge,
    #[error("the baseline observation queue no longer admits records")]
    Closed,
}

// A queued record keeps the slot it was admitted with until a flush acknowledges it.
pub struct QueuedRecord {
    pub record: BaselineAccountingRecord,
    pub slot: OwnedSemaphorePermit,
}

// Capacity counts leased records too, so a requeue never waits for a slot it already holds.
pub struct BaselineQueue {
    capacity: Arc<Semaphore>,
    pending: Mutex<VecDeque<QueuedRecord>>,
}

// The records one flush owns; whatever it still holds when dropped goes back behind the pending ones.
pub struct Lease<'queue> {
    pub queue: &'queue BaselineQueue,
    pub records: Vec<QueuedRecord>,
    pub more_queued: bool,
}

impl BaselineQueue {
    // The capacity is the spend queue's LITELLM_ASYNCIO_QUEUE_MAXSIZE that B1690 loads.
    pub fn new(capacity: usize) -> Result<Self, QueueError> {
        if capacity == 0 {
            return Err(QueueError::ZeroCapacity);
        }
        if capacity > Semaphore::MAX_PERMITS {
            return Err(QueueError::CapacityTooLarge);
        }
        Ok(Self {
            capacity: Arc::new(Semaphore::new(capacity)),
            pending: Mutex::new(VecDeque::new()),
        })
    }

    // A full queue makes the producer wait instead of dropping the observation.
    pub async fn push(&self, record: BaselineAccountingRecord) -> Result<(), QueueError> {
        let slot = Arc::clone(&self.capacity)
            .acquire_owned()
            .await
            .map_err(|_closed| QueueError::Closed)?;
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(QueuedRecord { record, slot });
        Ok(())
    }

    // Shutdown wakes producers still waiting for a slot, so none of them hangs on a queue nobody drains.
    #[must_use]
    pub fn close(&self) -> usize {
        self.capacity.close();
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    #[must_use]
    pub fn lease(&self, limit: usize) -> Lease<'_> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let count = limit.min(pending.len());
        let records = pending.drain(..count).collect();
        let more_queued = !pending.is_empty();
        drop(pending);
        Lease {
            queue: self,
            records,
            more_queued,
        }
    }
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        self.queue
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(self.records.drain(..));
    }
}
