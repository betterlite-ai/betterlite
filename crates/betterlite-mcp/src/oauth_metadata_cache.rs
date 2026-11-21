mod state;
mod tests;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_db::mcp_server_encoding::request::Maybe;
use betterlite_db::mcp_stored_contracts::JsonValue;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use tokio::time::Instant;

use self::state::{CacheEntry, CacheState};

pub const TTL_SECONDS: u64 = 300;
pub const NEGATIVE_TTL_SECONDS: u64 = 60;
pub const MAX_SIZE: usize = 128;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CacheKey {
    pub server_id: String,
    pub resource_url: String,
}

#[derive(Clone, Default)]
pub struct OAuthMetadataCache {
    inner: Arc<Mutex<CacheState>>,
}

impl OAuthMetadataCache {
    pub fn lookup(&self, key: &CacheKey, now: Instant) -> Maybe<JsonValue> {
        let state = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = state.entries.get(key)?;
        if entry.expires_at > now {
            return Some(entry.payload.clone());
        }
        None
    }

    pub fn prune(&self, now: Instant) {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .prune(now);
    }

    #[must_use]
    pub fn store(
        &self,
        key: &CacheKey,
        generation: u64,
        payload: Option<JsonValue>,
        ttl_seconds: u64,
        now: Instant,
    ) -> bool {
        let mut state = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if state.generations.get(&key.server_id).copied().unwrap_or(0) != generation {
            return false;
        }
        let Some(expires_at) = now.checked_add(Duration::from_secs(ttl_seconds)) else {
            return false;
        };
        // Re-storing a key keeps its first sequence, the way a Python dict assignment keeps the key's position
        let existing = state.entries.get(key).map(|entry| entry.sequence);
        let sequence = match existing {
            Some(sequence) => sequence,
            None => {
                let next = state.next_sequence;
                state.next_sequence = next.wrapping_add(1);
                next
            }
        };
        state.entries.insert(
            key.clone(),
            CacheEntry {
                expires_at,
                payload,
                sequence,
            },
        );
        state.prune(now);
        true
    }

    #[must_use]
    pub fn generation(&self, server_id: &str) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .generations
            .get(server_id)
            .copied()
            .unwrap_or(0)
    }

    #[must_use]
    pub async fn enter_fetch(&self, key: &CacheKey) -> FetchSlot {
        let lock = {
            let mut state = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
            let count = state.fetchers.entry(key.clone()).or_insert(0);
            *count = count.saturating_add(1);
            Arc::clone(state.locks.entry(key.clone()).or_default())
        };
        let mut slot = FetchSlot {
            guard: None,
            key: key.clone(),
            inner: Arc::clone(&self.inner),
        };
        slot.guard = Some(lock.lock_owned().await);
        slot
    }

    pub fn invalidate(&self, server_id: &str) {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .invalidate(server_id);
    }
}

pub struct FetchSlot {
    guard: Option<OwnedMutexGuard<()>>,
    key: CacheKey,
    inner: Arc<Mutex<CacheState>>,
}

impl Drop for FetchSlot {
    fn drop(&mut self) {
        drop(self.guard.take());
        let mut state = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let remaining = match state.fetchers.get_mut(&self.key) {
            Some(count) => {
                let lowered = count.saturating_sub(1);
                *count = lowered;
                lowered
            }
            None => 0,
        };
        if remaining == 0 {
            state.fetchers.remove(&self.key);
        }
    }
}
