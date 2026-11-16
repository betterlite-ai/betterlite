mod tests;

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::Mutex as AsyncMutex;
use tokio::time::Instant;

use super::{
    MAX_RECENT_MISSES, READ_THROUGH_MAX_RESYNCS, READ_THROUGH_MISS_TTL, READ_THROUGH_RESYNC_WINDOW,
    RecentMisses, RegistryReadThrough, Resync, State,
};

impl<R: Resync> RegistryReadThrough<R> {
    #[must_use]
    pub fn new(resync: R) -> Self {
        Self {
            resync,
            miss_ttl: READ_THROUGH_MISS_TTL,
            max_resyncs_per_window: READ_THROUGH_MAX_RESYNCS,
            window: READ_THROUGH_RESYNC_WINDOW,
            recent_misses: Mutex::new(RecentMisses::default()),
            state: AsyncMutex::new(State::default()),
        }
    }

    pub async fn attempt(&self, key: &str) -> bool {
        if self.is_recent_miss(key) {
            return false;
        }
        let mut state = self.state.lock().await;
        if self.is_recent_miss(key) {
            return false;
        }
        if !state.consume_resync_budget(self.max_resyncs_per_window, self.window) {
            tracing::warn!(
                key = %key,
                max_resyncs = self.max_resyncs_per_window,
                window_seconds = self.window.as_secs(),
                "registry read-through skipped: the resync budget is exhausted"
            );
            return false;
        }
        let outcome = self.resync.resync(key).await;
        if matches!(outcome, Ok(false)) {
            self.recent_misses
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remember(key, Instant::now(), self.miss_ttl);
        }
        drop(state);
        match outcome {
            Ok(found) => found,
            Err(error) => {
                tracing::warn!(key = %key, error = %error, "registry read-through failed");
                false
            }
        }
    }

    fn is_recent_miss(&self, key: &str) -> bool {
        self.recent_misses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(key, Instant::now())
    }
}

impl RecentMisses {
    fn contains(&self, key: &str, now: Instant) -> bool {
        let Some(expiry) = self.entries.get(key) else {
            return false;
        };
        now <= *expiry
    }

    // LiteLLM prunes expired entries and then the earliest expiry until one slot is free
    fn remember(&mut self, key: &str, now: Instant, ttl: Duration) {
        let Some(expiry) = now.checked_add(ttl) else {
            return;
        };
        self.entries.retain(|_, entry_expiry| now < *entry_expiry);
        while self.entries.len() >= MAX_RECENT_MISSES {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry_expiry)| **entry_expiry)
                .map(|(entry_key, _)| entry_key.clone());
            let Some(oldest_key) = oldest else {
                break;
            };
            self.entries.remove(&oldest_key);
        }
        self.entries.insert(key.to_owned(), expiry);
    }
}
