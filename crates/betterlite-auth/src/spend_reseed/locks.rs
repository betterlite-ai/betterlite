use std::sync::{Arc, Mutex, PoisonError};

use indexmap::IndexMap;
use tokio::sync::Notify;

// each counter's operation slot: the state names its owner, the notification wakes the waiters
pub struct KeyOperation {
    state: Mutex<OperationState>,
    released: Notify,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OperationState {
    Idle,
    Running,
}

pub struct LockRegistry {
    capacity: usize,
    // an ordered map keeps LiteLLM's OrderedDict recency: a hit moves to the end, overflow drops index 0
    operations: Mutex<IndexMap<String, Arc<KeyOperation>>>,
}

// held for the whole operation, but it is not a mutex guard: nothing blocks the task while it awaits
pub struct OperationGuard {
    pub operation: Arc<KeyOperation>,
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        let mut state = self
            .operation
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *state = OperationState::Idle;
        // released before the notification so a woken caller claims an idle slot, not a running one
        drop(state);
        self.operation.released.notify_waiters();
    }
}

impl From<i64> for LockRegistry {
    fn from(max_size: i64) -> Self {
        Self {
            capacity: usize::try_from(max_size).unwrap_or(0),
            operations: Mutex::new(IndexMap::new()),
        }
    }
}

impl LockRegistry {
    #[must_use]
    pub fn operation(&self, counter_key: &str) -> Arc<KeyOperation> {
        let mut operations = self
            .operations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some((key, held)) = operations.shift_remove_entry(counter_key) {
            drop(operations.insert(key, Arc::clone(&held)));
            return held;
        }
        let operation = Arc::new(KeyOperation {
            state: Mutex::new(OperationState::Idle),
            released: Notify::new(),
        });
        drop(operations.insert(counter_key.to_owned(), Arc::clone(&operation)));
        // a zero or negative bound strips every insert, so concurrent callers may run at once
        if operations.len() > self.capacity {
            drop(operations.shift_remove_index(0));
        }
        operation
    }

    pub async fn hold(&self, counter_key: &str) -> OperationGuard {
        loop {
            let operation = self.operation(counter_key);
            // the notification borrows the operation, so it is dropped before the guard takes ownership
            let acquired = {
                let released = operation.released.notified();
                tokio::pin!(released);
                // registering before the check is what makes a release that lands in between still wake us
                released.as_mut().enable();
                let idle = {
                    let mut state = operation
                        .state
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    if *state == OperationState::Idle {
                        *state = OperationState::Running;
                        true
                    } else {
                        false
                    }
                };
                if idle {
                    true
                } else {
                    released.await;
                    false
                }
            };
            if acquired {
                return OperationGuard { operation };
            }
        }
    }
}
