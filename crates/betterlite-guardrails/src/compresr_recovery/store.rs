use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use indexmap::IndexMap;

use super::hash::display_hash;

pub const ORIGINALS_TTL: Duration = Duration::from_mins(15);
pub const MAX_TRACKED_CALLS: usize = 256;
pub const MAX_TOTAL_STORE_BYTES: usize = 256 * 1024 * 1024;

pub type Originals = IndexMap<String, String>;

// The caller scope is the hash of the authenticated virtual key; the call id alone is client-settable
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StoreKey {
    pub caller_scope: String,
    pub call_id: String,
}

impl StoreKey {
    // No call id means no recovery for the call
    #[must_use]
    pub fn resolve(call_id: Option<&str>, caller_scope: &str) -> Option<Self> {
        let id = call_id.filter(|present| !present.is_empty())?;
        Some(Self {
            caller_scope: caller_scope.to_owned(),
            call_id: id.to_owned(),
        })
    }
}

#[must_use]
pub fn entry_bytes(originals: &Originals) -> usize {
    let mut total = 0_usize;
    for text in originals.values() {
        total = total.saturating_add(text.len());
    }
    total
}

#[derive(Debug)]
pub(super) struct Entry {
    pub(super) originals: Originals,
    pub(super) expires_at: Duration,
}

// Insertion order is expiry order, since every entry gets the same TTL, so pruning only reads the front
#[derive(Debug, Default)]
pub(super) struct State {
    pub(super) entries: IndexMap<StoreKey, Entry>,
    pub(super) total_bytes: usize,
}

impl State {
    pub(super) fn insert(&mut self, key: &StoreKey, originals: Originals, expires_at: Duration) {
        self.total_bytes = self.total_bytes.saturating_add(entry_bytes(&originals));
        self.entries.insert(
            key.clone(),
            Entry {
                originals,
                expires_at,
            },
        );
    }

    fn evict_oldest(&mut self) {
        if let Some((_, entry)) = self.entries.shift_remove_index(0) {
            self.total_bytes = self
                .total_bytes
                .saturating_sub(entry_bytes(&entry.originals));
        }
    }

    // The newest entry survives the byte ceiling so the current call's originals are kept
    pub(super) fn prune(&mut self, now: Duration, max_total_bytes: usize) {
        let expired = self
            .entries
            .values()
            .take_while(|entry| entry.expires_at <= now)
            .count();
        for _ in 0..expired {
            self.evict_oldest();
        }
        let over_calls = self.entries.len().saturating_sub(MAX_TRACKED_CALLS);
        for _ in 0..over_calls {
            self.evict_oldest();
        }
        // One pass over the entries other than the newest, ending as soon as the rest fit
        let evictable = self.entries.len().saturating_sub(1);
        for _ in 0..evictable {
            if self.total_bytes <= max_total_bytes {
                break;
            }
            self.evict_oldest();
        }
    }
}

// Oldest originals go first until the call fits; a cap of 0 turns the limit off
fn bound_call_bytes(mut merged: Originals, max_bytes_per_call: usize) -> Originals {
    if max_bytes_per_call == 0 {
        return merged;
    }
    let mut total = entry_bytes(&merged);
    while total > max_bytes_per_call {
        let Some((hash, text)) = merged.shift_remove_index(0) else {
            break;
        };
        total = total.saturating_sub(text.len());
        tracing::warn!(hash, "Compresr: originals-store byte cap hit, evicted");
    }
    merged
}

#[derive(Debug, Default)]
pub struct RecoveryStore {
    pub(super) state: Mutex<State>,
}

impl RecoveryStore {
    pub fn store(
        &self,
        key: &StoreKey,
        originals: Originals,
        now: Duration,
        max_bytes_per_call: usize,
    ) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut merged = if let Some(previous) = state.entries.shift_remove(key) {
            state.total_bytes = state
                .total_bytes
                .saturating_sub(entry_bytes(&previous.originals));
            previous.originals
        } else {
            Originals::new()
        };
        merged.extend(originals);
        let bounded = bound_call_bytes(merged, max_bytes_per_call);
        state.insert(key, bounded, now.saturating_add(ORIGINALS_TTL));
        state.prune(now, MAX_TOTAL_STORE_BYTES);
    }

    // The number of calls dropped
    #[must_use]
    pub fn prune(&self, now: Duration) -> usize {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let before = state.entries.len();
        state.prune(now, MAX_TOTAL_STORE_BYTES);
        before.saturating_sub(state.entries.len())
    }

    // What an earlier turn under the same key already stored, expired or not, for the per-call byte budget
    #[must_use]
    pub fn existing_originals(&self, key: Option<&StoreKey>) -> Originals {
        let Some(present) = key else {
            return Originals::new();
        };
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .entries
            .get(present)
            .map_or_else(Originals::new, |entry| entry.originals.clone())
    }

    // None when the hash was not issued under this key, or its entry has expired
    #[must_use]
    pub fn retrieve(&self, key: Option<&StoreKey>, hash: &str, now: Duration) -> Option<String> {
        let present = key?;
        let found = {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state
                .entries
                .get(present)
                .filter(|entry| entry.expires_at > now)
                .and_then(|entry| entry.originals.get(hash))
                .cloned()
        };
        if found.is_none() {
            tracing::warn!(
                hash = %display_hash(hash),
                "Compresr retrieve: rejecting a hash that was not issued for this request, or has expired"
            );
        }
        found
    }
}
