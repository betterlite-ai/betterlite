use std::collections::HashMap;
use std::sync::Arc;

use betterlite_db::mcp_stored_contracts::JsonValue;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::Instant;

use super::{CacheKey, MAX_SIZE};

pub(super) struct CacheEntry {
    pub(super) expires_at: Instant,
    pub(super) payload: Option<JsonValue>,
    pub(super) sequence: u64,
}

#[derive(Default)]
pub(super) struct CacheState {
    pub(super) entries: HashMap<CacheKey, CacheEntry>,
    pub(super) locks: HashMap<CacheKey, Arc<AsyncMutex<()>>>,
    pub(super) fetchers: HashMap<CacheKey, usize>,
    pub(super) generations: HashMap<String, u64>,
    pub(super) next_sequence: u64,
}

impl CacheState {
    pub(super) fn prune(&mut self, now: Instant) {
        self.entries.retain(|_, entry| entry.expires_at > now);
        let overflow = self.entries.len().saturating_sub(MAX_SIZE);
        if overflow > 0 {
            let mut by_expiry: Vec<(&CacheKey, &CacheEntry)> = self.entries.iter().collect();
            by_expiry.sort_by_key(|(_, entry)| (entry.expires_at, entry.sequence));
            let evicted: Vec<CacheKey> = by_expiry
                .into_iter()
                .take(overflow)
                .map(|(key, _)| key.clone())
                .collect();
            for key in evicted {
                self.entries.remove(&key);
            }
        }
        // Lock.locked() reads false between one holder's release and the next waiter waking, so the fetcher count decides idleness
        let idle: Vec<CacheKey> = self
            .locks
            .keys()
            .filter(|key| !self.entries.contains_key(*key) && self.lock_idle(key))
            .cloned()
            .collect();
        for key in idle {
            self.locks.remove(&key);
        }
        let in_flight: Vec<&str> = self
            .fetchers
            .keys()
            .map(|key| key.server_id.as_str())
            .collect();
        self.generations
            .retain(|server_id, _| in_flight.contains(&server_id.as_str()));
    }

    pub(super) fn invalidate(&mut self, server_id: &str) {
        if self.fetchers.keys().any(|key| key.server_id == server_id) {
            let generation = self.generations.get(server_id).copied().unwrap_or(0);
            self.generations
                .insert(server_id.to_owned(), generation.wrapping_add(1));
        } else {
            self.generations.remove(server_id);
        }
        self.entries.retain(|key, _| key.server_id != server_id);
        let idle: Vec<CacheKey> = self
            .locks
            .keys()
            .filter(|key| key.server_id == server_id && self.lock_idle(key))
            .cloned()
            .collect();
        for key in idle {
            self.locks.remove(&key);
        }
    }

    fn lock_idle(&self, key: &CacheKey) -> bool {
        if self.fetchers.contains_key(key) {
            return false;
        }
        self.locks
            .get(key)
            .is_none_or(|lock| lock.try_lock().is_ok())
    }
}
