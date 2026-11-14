use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

// LiteLLM's _HASH_CACHE_TTL_SECONDS: 15 * 60
pub(crate) const HASH_CACHE_TTL: Duration = Duration::from_secs(900);

#[derive(Debug, Clone)]
struct IssuedHashes {
    hashes: BTreeSet<String>,
    expires_at: Duration,
}

#[derive(Debug, Default)]
pub(crate) struct IssuedHashStore {
    entries: Mutex<BTreeMap<String, IssuedHashes>>,
}

impl IssuedHashStore {
    pub(crate) fn prune_expired(&self, now: Duration) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.retain(|_call_id, issued| issued.expires_at > now);
    }

    pub(crate) fn record(&self, call_id: &str, hashes: BTreeSet<String>, now: Duration) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let issued = IssuedHashes {
            hashes,
            expires_at: now.saturating_add(HASH_CACHE_TTL),
        };
        entries.insert(call_id.to_owned(), issued);
    }

    pub(crate) fn lookup(&self, call_id: &str) -> Option<BTreeSet<String>> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.get(call_id).map(|issued| issued.hashes.clone())
    }
}
