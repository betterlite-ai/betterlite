use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use betterlite_mcp::credential_contracts::errors::{CredError, CredResult};
use chrono::{DateTime, TimeDelta, Utc};
use indexmap::IndexMap;

use super::ExchangedToken;
use crate::mcp::outbound_credentials::token_exchange_provider::TokenExchangeTunables;
use crate::mcp::outbound_credentials::{RefreshFuture, WallClock};

struct CacheEntry {
    fingerprint: String,
    token: String,
    valid_until: DateTime<Utc>,
}

// The lock and the generation an overlapping invalidate bumps sit on one object, so a compute snapshots the generation of the very guard its caller holds.
struct KeyGuard {
    single_flight: tokio::sync::Mutex<()>,
    generation: Mutex<u64>,
}

pub struct ExchangedTokenCache {
    entries: Mutex<IndexMap<String, CacheEntry>>,
    guards: Mutex<HashMap<String, Weak<KeyGuard>>>,
    clock: Arc<dyn WallClock>,
    tunables: TokenExchangeTunables,
}

impl ExchangedTokenCache {
    #[must_use]
    pub fn new(clock: Arc<dyn WallClock>, tunables: TokenExchangeTunables) -> Self {
        Self {
            entries: Mutex::new(IndexMap::new()),
            guards: Mutex::new(HashMap::new()),
            clock,
            tunables,
        }
    }

    pub async fn get_or_compute(
        &self,
        cache_key: &str,
        fingerprint: &str,
        compute: RefreshFuture<'_, ExchangedToken, CredError>,
    ) -> CredResult<String> {
        if let Some(token) = self.get(cache_key, fingerprint) {
            return Ok(token);
        }
        let guard = self.guard(cache_key);
        let _held = guard.single_flight.lock().await;
        if let Some(token) = self.get(cache_key, fingerprint) {
            return Ok(token);
        }
        let generation = Self::generation(&guard);
        let token = compute.await?;
        if Self::generation(&guard) == generation {
            self.store(cache_key, fingerprint, &token);
        }
        Ok(token.access_token)
    }

    // An invalidate that lands while a compute awaits the endpoint wins over that compute's write.
    pub fn invalidate(&self, cache_key: &str) {
        drop(
            self.entries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .shift_remove(cache_key),
        );
        let guard = self
            .guards
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(cache_key)
            .and_then(Weak::upgrade);
        if let Some(guard) = guard {
            let mut generation = guard
                .generation
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            *generation = generation.saturating_add(1);
        }
    }

    fn generation(guard: &KeyGuard) -> u64 {
        *guard
            .generation
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn get(&self, cache_key: &str, fingerprint: &str) -> Option<String> {
        let now = self.clock.now();
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = entries.get(cache_key)?;
        if now < entry.valid_until {
            return (entry.fingerprint == fingerprint).then(|| entry.token.clone());
        }
        drop(entries.shift_remove(cache_key));
        None
    }

    fn store(&self, cache_key: &str, fingerprint: &str, token: &ExchangedToken) {
        if self.tunables.cache_max_size == 0 {
            return;
        }
        let ttl = cache_ttl_seconds(token.expires_in, &self.tunables);
        let lifetime = TimeDelta::from_std(ttl).unwrap_or(TimeDelta::MAX);
        let valid_until = self
            .clock
            .now()
            .checked_add_signed(lifetime)
            .unwrap_or(DateTime::<Utc>::MAX_UTC);
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        // LiteLLM's InMemoryCache evicts the soonest-expiring entry, so a live token is what leaves the cache.
        if entries.len() >= self.tunables.cache_max_size {
            let soonest = entries
                .iter()
                .min_by_key(|(key, entry)| (entry.valid_until, key.as_str()))
                .map(|(key, _entry)| key.clone());
            if let Some(soonest) = soonest {
                drop(entries.shift_remove(&soonest));
            }
        }
        entries.insert(
            cache_key.to_owned(),
            CacheEntry {
                fingerprint: fingerprint.to_owned(),
                token: token.access_token.clone(),
                valid_until,
            },
        );
    }

    fn guard(&self, cache_key: &str) -> Arc<KeyGuard> {
        let mut guards = self.guards.lock().unwrap_or_else(PoisonError::into_inner);
        guards.retain(|_, weak| weak.strong_count() > 0);
        if let Some(existing) = guards.get(cache_key).and_then(Weak::upgrade) {
            return existing;
        }
        let guard = Arc::new(KeyGuard {
            single_flight: tokio::sync::Mutex::new(()),
            generation: Mutex::new(0),
        });
        guards.insert(cache_key.to_owned(), Arc::downgrade(&guard));
        guard
    }
}

#[must_use]
pub fn cache_ttl_seconds(expires_in: Option<i64>, tunables: &TokenExchangeTunables) -> Duration {
    let lifetime = expires_in.map_or(tunables.default_ttl, |seconds| {
        Duration::from_secs(u64::try_from(seconds).unwrap_or(0))
    });
    lifetime
        .saturating_sub(tunables.expiry_buffer)
        .max(tunables.min_ttl)
}
