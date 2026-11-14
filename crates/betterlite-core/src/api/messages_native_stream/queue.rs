use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::Notify;

pub(super) enum QueueItem<E> {
    Chunk(Vec<u8>),
    End,
    Failed(Arc<E>),
}

pub(super) struct State<E> {
    pub(super) items: VecDeque<QueueItem<E>>,
    pub(super) closed: bool,
}

// The bounded client queue LiteLLM builds with asyncio.Queue(maxsize=...), plus its detach event
pub(super) struct ClientQueue<E> {
    pub(super) capacity: usize,
    pub(super) state: Mutex<State<E>>,
    pub(super) data: Notify,
    pub(super) space: Notify,
    pub(super) client_detached: AtomicBool,
    pub(super) detach: Notify,
}

impl<E> ClientQueue<E> {
    pub(super) fn detach(&self) {
        self.client_detached.store(true, Ordering::SeqCst);
        self.detach.notify_one();
        self.space.notify_one();
    }

    pub(super) fn close(&self) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed = true;
        self.data.notify_one();
    }

    // The guard lives in the block, so its storage ends before the wait and the future stays Send
    pub(super) async fn next(&self) -> Option<QueueItem<E>>
    where
        E: Send + Sync,
    {
        loop {
            let (item, closed) = {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                (state.items.pop_front(), state.closed)
            };
            if let Some(queued) = item {
                self.space.notify_one();
                return Some(queued);
            }
            if closed {
                return None;
            }
            self.data.notified().await;
        }
    }

    // Python's _enqueue_for_client: a full queue waits on space or detach, and a completed put wins
    pub(super) async fn enqueue(&self, mut item: QueueItem<E>) -> bool
    where
        E: Send + Sync,
    {
        if self.client_detached.load(Ordering::SeqCst) {
            return false;
        }
        while let Err(returned) = self.push_if_room(item) {
            item = returned;
            if self.client_detached.load(Ordering::SeqCst) {
                return false;
            }
            // a detach notifies the same wait, so the next turn takes the false branch above
            self.space.notified().await;
        }
        self.data.notify_one();
        true
    }

    // One put attempt under the lock; the item comes back while the queue is full
    fn push_if_room(&self, item: QueueItem<E>) -> Result<(), QueueItem<E>> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.items.len() >= self.capacity {
            return Err(item);
        }
        state.items.push_back(item);
        drop(state);
        Ok(())
    }

    pub(super) async fn wait_detached(&self)
    where
        E: Send + Sync,
    {
        let notified = self.detach.notified();
        if self.client_detached.load(Ordering::SeqCst) {
            return;
        }
        notified.await;
    }

    // Python's _exception_left_unconsumed: drain everything left and match that one exception
    pub(super) fn take_all_contains(&self, error: &Arc<E>) -> bool {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut found = false;
        while let Some(item) = state.items.pop_front() {
            if let QueueItem::Failed(queued) = item
                && Arc::ptr_eq(&queued, error)
            {
                found = true;
            }
        }
        drop(state);
        self.space.notify_one();
        found
    }
}
