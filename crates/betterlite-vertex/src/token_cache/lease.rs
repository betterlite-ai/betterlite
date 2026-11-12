use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::key::CredentialKey;

// LiteLLM's per-key refresh lock: one refresh runs while every other caller of that key waits
#[derive(Default)]
pub struct LeaseMap {
    pub(super) entries: Mutex<BTreeMap<CredentialKey, LeaseEntry>>,
}

pub(super) struct LeaseEntry {
    semaphore: Arc<Semaphore>,
    waiters: usize,
}

// a caller counts as a waiter from the moment it asks, so a cancelled wait still leaves the entry clean
pub(super) struct LeaseHold {
    leases: Arc<LeaseMap>,
    key: CredentialKey,
    pub(super) semaphore: Arc<Semaphore>,
}

impl Drop for LeaseHold {
    fn drop(&mut self) {
        self.leases.release(&self.key);
    }
}

impl LeaseMap {
    pub(super) fn hold(self: &Arc<Self>, key: &CredentialKey) -> LeaseHold {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = entries.entry(key.clone()).or_insert_with(|| LeaseEntry {
            semaphore: Arc::new(Semaphore::new(1)),
            waiters: 0,
        });
        entry.waiters = entry.waiters.saturating_add(1);
        let semaphore = Arc::clone(&entry.semaphore);
        drop(entries);
        LeaseHold {
            leases: Arc::clone(self),
            key: key.clone(),
            semaphore,
        }
    }

    fn release(&self, key: &CredentialKey) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(entry) = entries.get_mut(key) else {
            return;
        };
        entry.waiters = entry.waiters.saturating_sub(1);
        // LiteLLM drops the lock entry once no caller holds or waits on it
        if entry.waiters == 0 {
            entries.remove(key);
        }
    }
}

// the refresh lease is a semaphore permit, not a mutex guard, so a waiter parks instead of blocking a thread
pub struct RefreshLease {
    pub(super) _permit: OwnedSemaphorePermit,
    pub(super) _hold: LeaseHold,
}
