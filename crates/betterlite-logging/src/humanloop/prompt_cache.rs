use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_models::prompt_contracts::PromptManagementClient;

pub const MAX_CACHED_PROMPTS: usize = 200;

pub trait Clock {
    fn now(&self) -> Duration;
}

#[derive(Debug)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
    }
}

#[derive(Debug)]
pub struct CacheEntry {
    pub prompt: PromptManagementClient,
    pub expiry: Duration,
}

#[derive(Debug)]
pub struct PromptCache<C> {
    pub clock: C,
    pub ttl: Duration,
    pub entries: HashMap<String, CacheEntry>,
}

impl<C: Clock> PromptCache<C> {
    pub fn get_or_fetch<E, F>(
        &mut self,
        key: &str,
        fetch: F,
    ) -> Result<&mut PromptManagementClient, E>
    where
        F: FnOnce() -> Result<PromptManagementClient, E>,
    {
        let now = self.clock.now();
        let live = self
            .entries
            .get(key)
            .is_some_and(|entry| now <= entry.expiry);
        if !live {
            // Pruning here also removes an expired copy, so the entry below is vacant.
            self.make_room(now);
        }
        let entry = match self.entries.entry(key.to_owned()) {
            Entry::Occupied(occupied) => occupied.into_mut(),
            Entry::Vacant(vacant) => {
                let prompt = fetch()?;
                vacant.insert(CacheEntry {
                    prompt,
                    expiry: now.saturating_add(self.ttl),
                })
            }
        };
        Ok(&mut entry.prompt)
    }

    // Mirrors InMemoryCache.evict_cache: expired entries go first, then the earliest expiry while at capacity.
    fn make_room(&mut self, now: Duration) {
        self.entries.retain(|_key, entry| entry.expiry > now);
        while self.entries.len() >= MAX_CACHED_PROMPTS {
            let oldest = self
                .entries
                .iter()
                .min_by(|(left_key, left), (right_key, right)| {
                    left.expiry
                        .cmp(&right.expiry)
                        .then_with(|| left_key.cmp(right_key))
                })
                .map(|(key, _entry)| key.clone());
            let Some(oldest_key) = oldest else {
                break;
            };
            if self.entries.remove(&oldest_key).is_none() {
                break;
            }
        }
    }
}
