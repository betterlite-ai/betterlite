mod tests;

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::credential_contracts::byok_cache::Clock;
use crate::sso_assertions::assertion::SsoIdentityAssertion;

// The epoch and the entries move together: an invalidate bumps the epoch and drops the entry under one lock.
#[derive(Default)]
pub struct CacheState {
    pub epoch: u64,
    pub entries: HashMap<String, (SsoIdentityAssertion, Instant)>,
}

pub struct SsoAssertionCache {
    pub ttl: Duration,
    pub max_size: usize,
    pub clock: Box<dyn Clock>,
    pub state: Mutex<CacheState>,
}

impl SsoAssertionCache {
    pub fn epoch(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .epoch
    }

    pub fn get(&self, user_id: &str) -> Option<SsoIdentityAssertion> {
        let now = self.clock.now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let live = state
            .entries
            .get(user_id)
            .is_some_and(|(_, stored_at)| now.saturating_duration_since(*stored_at) < self.ttl);
        if !live {
            // LiteLLM drops an expired entry on the read that finds it, so the miss also frees the slot
            state.entries.remove(user_id);
            return None;
        }
        state
            .entries
            .get(user_id)
            .map(|(assertion, _)| assertion.clone())
    }

    pub fn set_if_unchanged(
        &self,
        user_id: &str,
        assertion: &SsoIdentityAssertion,
        seen_epoch: u64,
    ) {
        let now = self.clock.now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.epoch != seen_epoch {
            return;
        }
        if self.max_size == 0 {
            return;
        }
        // a write first clears every expired entry, so it never spends the room a live entry still needs
        state
            .entries
            .retain(|_, (_, stored_at)| now.saturating_duration_since(*stored_at) < self.ttl);
        while state.entries.len() >= self.max_size {
            let soonest = state
                .entries
                .iter()
                .min_by(|(left_key, (_, left_at)), (right_key, (_, right_at))| {
                    left_at.cmp(right_at).then_with(|| left_key.cmp(right_key))
                })
                .map(|(key, _)| key.clone());
            let Some(oldest) = soonest else {
                break;
            };
            state.entries.remove(&oldest);
        }
        // a live key keeps the expiry it already had; only an absent or just-dropped one starts a new lifetime
        let stored_at = state
            .entries
            .get(user_id)
            .map_or(now, |(_, stored_at)| *stored_at);
        state
            .entries
            .insert(user_id.to_owned(), (assertion.clone(), stored_at));
    }

    pub fn invalidate(&self, user_id: &str) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        // the bump lands before the removal, so no reader sees the entry gone under the old epoch
        state.epoch = state.epoch.wrapping_add(1);
        state.entries.remove(user_id);
    }
}
