use std::sync::{Arc, Mutex, PoisonError};

use indexmap::IndexMap;
use tokio::sync::Mutex as AsyncMutex;

pub struct LockRegistry {
    capacity: usize,
    // an ordered map keeps LiteLLM's OrderedDict recency: a hit moves to the end, overflow drops index 0
    locks: Mutex<IndexMap<String, Arc<AsyncMutex<()>>>>,
}

impl From<i64> for LockRegistry {
    fn from(max_size: i64) -> Self {
        Self {
            capacity: usize::try_from(max_size).unwrap_or(0),
            locks: Mutex::new(IndexMap::new()),
        }
    }
}

impl LockRegistry {
    #[must_use]
    pub fn lock_for(&self, counter_key: &str) -> Arc<AsyncMutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((key, held)) = locks.shift_remove_entry(counter_key) {
            locks.insert(key, Arc::clone(&held));
            return held;
        }
        let lock = Arc::new(AsyncMutex::new(()));
        locks.insert(counter_key.to_owned(), Arc::clone(&lock));
        // a zero or negative bound strips every insert, so concurrent callers may hold two locks
        if locks.len() > self.capacity {
            locks.shift_remove_index(0);
        }
        lock
    }
}
