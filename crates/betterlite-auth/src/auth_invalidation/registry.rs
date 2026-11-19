use std::sync::Mutex;

use betterlite_cache::dual_cache::limited_size::LimitedSizeOrderedDict;
use indexmap::IndexMap;
use tokio::sync::watch;
use tokio::task::Id;

// auth_checks.py's _TEAM_MEMBERSHIP_INFLIGHT_MAX: the index keeps the newest loads and drops the oldest
pub const MAX_MEMBERSHIP_LOADS: usize = 10_000;

// one installable load: the task's id, so a reset can skip waiting on itself, and the channel the load
// closes when it finishes, which every waiter may watch without taking the result away from the others
pub struct InflightLoad<Load> {
    pub id: Id,
    pub completion: watch::Receiver<Option<Load>>,
}

impl<Load> Clone for InflightLoad<Load> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            completion: self.completion.clone(),
        }
    }
}

pub struct MembershipInflight<Load> {
    entries: Mutex<LimitedSizeOrderedDict<InflightLoad<Load>>>,
}

impl<Load> Default for MembershipInflight<Load> {
    fn default() -> Self {
        Self {
            entries: Mutex::new(LimitedSizeOrderedDict {
                entries: IndexMap::new(),
                max_size: MAX_MEMBERSHIP_LOADS,
            }),
        }
    }
}

impl<Load> MembershipInflight<Load> {
    pub fn install(&self, key: &str, load: InflightLoad<Load>) {
        let mut entries = match self.entries.lock() {
            Ok(acquired) => acquired,
            Err(poisoned) => poisoned.into_inner(),
        };
        entries.insert(key.to_owned(), load);
    }

    pub fn get(&self, key: &str) -> Option<InflightLoad<Load>> {
        let entries = match self.entries.lock() {
            Ok(acquired) => acquired,
            Err(poisoned) => poisoned.into_inner(),
        };
        entries.entries.get(key).cloned()
    }

    pub fn pop(&self, key: &str) -> Option<InflightLoad<Load>> {
        let mut entries = match self.entries.lock() {
            Ok(acquired) => acquired,
            Err(poisoned) => poisoned.into_inner(),
        };
        entries.entries.shift_remove(key)
    }
}
