use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use tokio::sync::Notify;

#[derive(Debug, thiserror::Error)]
pub enum UsageQueueError {
    #[error("the request usage queue needs a capacity of at least one record")]
    ZeroCapacity,
    #[error("the request usage queue no longer admits records")]
    Closed,
}

// A request-time analytics buffer whose pending count the census reads; LiteLLM's list grows without a limit, while this one waits for room.
pub struct UsageQueue<T> {
    capacity: usize,
    state: Mutex<QueueState<T>>,
    room: Notify,
}

struct QueueState<T> {
    pending: VecDeque<T>,
    closed: bool,
}

// The outcome of one admission attempt, with the record handed back while there is no room for it.
enum Admission<T> {
    Queued,
    Full(T),
    Closed,
}

impl<T> UsageQueue<T> {
    // The capacity is the spend queue's LITELLM_ASYNCIO_QUEUE_MAXSIZE that B1690 loads.
    pub fn new(queue_capacity: i64) -> Result<Self, UsageQueueError> {
        let Ok(capacity) = usize::try_from(queue_capacity) else {
            return Err(UsageQueueError::ZeroCapacity);
        };
        if capacity == 0 {
            return Err(UsageQueueError::ZeroCapacity);
        }
        Ok(Self {
            capacity,
            state: Mutex::new(QueueState {
                pending: VecDeque::new(),
                closed: false,
            }),
            room: Notify::new(),
        })
    }

    // A full queue makes the producer wait with its record still in hand, so a cancelled wait loses nothing.
    pub async fn push(&self, item: T) -> Result<(), UsageQueueError> {
        let mut waiting = item;
        loop {
            let room = self.room.notified();
            tokio::pin!(room);
            // register before the check, so a drain that frees a slot right after cannot miss this waiter
            room.as_mut().enable();
            match self.try_push(waiting) {
                Admission::Queued => return Ok(()),
                Admission::Closed => return Err(UsageQueueError::Closed),
                Admission::Full(returned) => waiting = returned,
            }
            room.await;
        }
    }

    fn try_push(&self, item: T) -> Admission<T> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.closed {
            return Admission::Closed;
        }
        if state.pending.len() < self.capacity {
            state.pending.push_back(item);
            return Admission::Queued;
        }
        Admission::Full(item)
    }

    // drain: hand over the oldest records up to the limit and wake every producer waiting for room.
    #[must_use]
    pub fn drain(&self, limit: usize) -> Vec<T> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let taken = state.pending.len().min(limit);
        let drained: Vec<T> = state.pending.drain(..taken).collect();
        drop(state);
        self.room.notify_waiters();
        drained
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pending
            .len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pending
            .is_empty()
    }

    // Shutdown wakes producers still waiting for a slot, so none of them hangs on a queue nobody drains.
    #[must_use]
    pub fn close(&self) -> usize {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.closed = true;
        let pending = state.pending.len();
        drop(state);
        self.room.notify_waiters();
        pending
    }
}
