use std::mem::take;
use std::sync::{Arc, Mutex, PoisonError};

use super::export::VantageExportLogger;

#[path = "store/tests/dedup.rs"]
mod dedup;
#[path = "store/tests/order.rs"]
mod order;
#[path = "store/tests/removal.rs"]
mod removal;

// LiteLLM's _get_all_callbacks order: the general list, success, failure, then the async success and failure lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VantageCallbackList {
    Callbacks,
    Success,
    Failure,
    AsyncSuccess,
    AsyncFailure,
}

struct Lists {
    callbacks: Vec<Arc<dyn VantageExportLogger>>,
    success: Vec<Arc<dyn VantageExportLogger>>,
    failure: Vec<Arc<dyn VantageExportLogger>>,
    async_success: Vec<Arc<dyn VantageExportLogger>>,
    async_failure: Vec<Arc<dyn VantageExportLogger>>,
}

impl Lists {
    // false when a logger with the same registration key is already in this list: the first one keeps its place
    fn push(&mut self, list: VantageCallbackList, logger: Arc<dyn VantageExportLogger>) -> bool {
        let key = logger.registration_identity().key();
        let registered = match list {
            VantageCallbackList::Callbacks => &mut self.callbacks,
            VantageCallbackList::Success => &mut self.success,
            VantageCallbackList::Failure => &mut self.failure,
            VantageCallbackList::AsyncSuccess => &mut self.async_success,
            VantageCallbackList::AsyncFailure => &mut self.async_failure,
        };
        if registered
            .iter()
            .any(|stored| stored.registration_identity().key() == key)
        {
            return false;
        }
        registered.push(logger);
        true
    }

    fn take(&mut self, list: VantageCallbackList) -> Vec<Arc<dyn VantageExportLogger>> {
        let registered = match list {
            VantageCallbackList::Callbacks => &mut self.callbacks,
            VantageCallbackList::Success => &mut self.success,
            VantageCallbackList::Failure => &mut self.failure,
            VantageCallbackList::AsyncSuccess => &mut self.async_success,
            VantageCallbackList::AsyncFailure => &mut self.async_failure,
        };
        take(registered)
    }

    // one instance per registered logger, first occurrence winning, in _get_all_callbacks order
    fn ordered(&self) -> Vec<Arc<dyn VantageExportLogger>> {
        let mut snapshot: Vec<Arc<dyn VantageExportLogger>> = Vec::new();
        for logger in self
            .callbacks
            .iter()
            .chain(self.success.iter())
            .chain(self.failure.iter())
            .chain(self.async_success.iter())
            .chain(self.async_failure.iter())
        {
            if !snapshot.iter().any(|seen| Arc::ptr_eq(seen, logger)) {
                snapshot.push(Arc::clone(logger));
            }
        }
        snapshot
    }
}

// The live Vantage instances and the callback lists each one sits in, shared as one Arc by every consumer.
pub struct VantageLoggers {
    lists: Mutex<Lists>,
}

impl VantageLoggers {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            lists: Mutex::new(Lists {
                callbacks: Vec::new(),
                success: Vec::new(),
                failure: Vec::new(),
                async_success: Vec::new(),
                async_failure: Vec::new(),
            }),
        }
    }

    pub fn register(&self, logger: Arc<dyn VantageExportLogger>, list: VantageCallbackList) {
        let mut lists = self.lists.lock().unwrap_or_else(PoisonError::into_inner);
        if !lists.push(list, logger) {
            tracing::debug!(
                list = ?list,
                "callback already registered in this list; keeping the first"
            );
        }
    }

    // the callers drop the lock before they await: every call hands out its own ordered copy
    #[must_use]
    pub fn snapshot(&self) -> Vec<Arc<dyn VantageExportLogger>> {
        let lists = self.lists.lock().unwrap_or_else(PoisonError::into_inner);
        lists.ordered()
    }

    // LiteLLM's remove_callbacks_by_type over one list: the instances in the other lists stay selectable
    #[must_use]
    pub fn remove_from_list(&self, list: VantageCallbackList) -> usize {
        let mut lists = self.lists.lock().unwrap_or_else(PoisonError::into_inner);
        lists.take(list).len()
    }
}

impl Default for VantageLoggers {
    fn default() -> Self {
        Self::new()
    }
}
