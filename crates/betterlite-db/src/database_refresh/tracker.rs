use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TransactionId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainStatus {
    pub active_operations: usize,
    pub transactions: usize,
    pub drained: bool,
}

struct DrainState {
    active_operations: usize,
    transactions: BTreeSet<TransactionId>,
    next_transaction: u64,
}

pub struct DrainTracker {
    state: Mutex<DrainState>,
    drained: watch::Sender<bool>,
}

impl DrainTracker {
    #[must_use]
    pub fn new() -> Self {
        let (drained, _receiver) = watch::channel(true);
        Self {
            state: Mutex::new(DrainState {
                active_operations: 0,
                transactions: BTreeSet::new(),
                next_transaction: 0,
            }),
            drained,
        }
    }

    #[must_use]
    pub fn status(&self) -> DrainStatus {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        DrainStatus {
            active_operations: state.active_operations,
            transactions: state.transactions.len(),
            drained: state.active_operations == 0,
        }
    }

    pub async fn wait_until_drained(&self) {
        match self.drained.subscribe().wait_for(|value| *value).await {
            Ok(_drained) => {}
            Err(closed) => tracing::debug!("database drain tracking closed: {closed}"),
        }
    }

    pub fn begin_operation(self: &Arc<Self>) -> OperationGuard {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.active_operations = state.active_operations.saturating_add(1);
        self.drained.send_replace(false);
        drop(state);
        OperationGuard {
            tracker: Arc::clone(self),
        }
    }

    pub fn begin_transaction(self: &Arc<Self>) -> TransactionHold {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.active_operations = state.active_operations.saturating_add(1);
        let id = TransactionId(state.next_transaction);
        state.next_transaction = state.next_transaction.wrapping_add(1);
        state.transactions.insert(id);
        self.drained.send_replace(false);
        drop(state);
        TransactionHold {
            id,
            tracker: Arc::clone(self),
        }
    }

    pub fn finish_transaction(&self, id: TransactionId) -> bool {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if !state.transactions.remove(&id) {
            return false;
        }
        state.active_operations = state.active_operations.saturating_sub(1);
        self.drained.send_replace(state.active_operations == 0);
        true
    }

    fn end_operation(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.active_operations = state.active_operations.saturating_sub(1);
        self.drained.send_replace(state.active_operations == 0);
    }
}

impl Default for DrainTracker {
    fn default() -> Self {
        Self::new()
    }
}

pub struct OperationGuard {
    tracker: Arc<DrainTracker>,
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        self.tracker.end_operation();
    }
}

pub struct TransactionHold {
    pub(crate) id: TransactionId,
    pub(crate) tracker: Arc<DrainTracker>,
}

impl Drop for TransactionHold {
    fn drop(&mut self) {
        self.tracker.finish_transaction(self.id);
    }
}
