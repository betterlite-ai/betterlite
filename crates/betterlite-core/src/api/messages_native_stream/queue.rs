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
    }

    pub(super) fn close(&self) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed = true;
        self.data.notify_one();
    }

    pub(super) async fn next(&self) -> Option<QueueItem<E>> {
        loop {
            let closed = {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some(item) = state.items.pop_front() {
                    drop(state);
                    self.space.notify_one();
                    return Some(item);
                }
                state.closed
            };
            if closed {
                return None;
            }
            self.data.notified().await;
        }
    }

    // Python's _enqueue_for_client: a full queue waits on space and the detach signal, and a
    // completed put wins even when the detach is ready too
    pub(super) async fn enqueue(&self, mut item: QueueItem<E>) -> bool {
        if self.client_detached.load(Ordering::SeqCst) {
            return false;
        }
        loop {
            {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                if state.items.len() < self.capacity {
                    state.items.push_back(item);
                    drop(state);
                    self.data.notify_one();
                    return true;
                }
            }
            if self.client_detached.load(Ordering::SeqCst) {
                return false;
            }
            tokio::select! {
                biased;
                () = self.space.notified() => {}
                () = self.detach.notified() => {}
            }
        }
    }

    pub(super) async fn wait_detached(&self) {
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
