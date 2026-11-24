use std::collections::BTreeMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use secrecy::SecretString;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::Instant;

use crate::credential_contracts::errors::CredResult;

use super::collaborators::ExchangedToken;

const MAX_SIZE: usize = 500;
const DEFAULT_TTL_SECONDS: i64 = 3600;
const EXPIRY_BUFFER_SECONDS: i64 = 60;
const MIN_TTL_SECONDS: i64 = 10;

struct Guard {
    lock: AsyncMutex<()>,
    generation: AtomicU64,
}

struct Entry {
    fingerprint: String,
    access_token: SecretString,
    expires_at: Instant,
}

#[derive(Default)]
struct CacheState {
    entries: BTreeMap<String, Entry>,
    // A weak reference keeps an invalidate able to fence the compute already holding the guard, without pinning every key ever seen.
    guards: BTreeMap<String, Weak<Guard>>,
}

#[derive(Default)]
pub struct ExchangedTokenCache {
    inner: Arc<Mutex<CacheState>>,
}

impl ExchangedTokenCache {
    pub async fn get_or_compute<F>(
        &self,
        cache_key: &str,
        fingerprint: &str,
        compute: F,
    ) -> CredResult<SecretString>
    where
        F: Future<Output = CredResult<ExchangedToken>> + Send,
    {
        if let Some(cached) = self.cached(cache_key, fingerprint) {
            return Ok(cached);
        }
        let guard = self.guard(cache_key);
        let _permit = guard.lock.lock().await;
        if let Some(cached) = self.cached(cache_key, fingerprint) {
            return Ok(cached);
        }
        let generation = guard.generation.load(Ordering::SeqCst);
        let token = compute.await?;
        if guard.generation.load(Ordering::SeqCst) == generation {
            self.store(cache_key, fingerprint, &token);
        }
        Ok(token.access_token)
    }

    pub fn invalidate(&self, cache_key: &str) {
        let mut state = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        state.entries.remove(cache_key);
        let guard = state
            .guards
            .remove(cache_key)
            .and_then(|weak| weak.upgrade());
        drop(state);
        if let Some(held) = guard {
            held.generation.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn cached(&self, cache_key: &str, fingerprint: &str) -> Option<SecretString> {
        let now = Instant::now();
        let mut state = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = state.entries.get(cache_key)?;
        if entry.expires_at <= now {
            state.entries.remove(cache_key);
            drop(state);
            return None;
        }
        if entry.fingerprint == fingerprint {
            return Some(entry.access_token.clone());
        }
        None
    }

    fn guard(&self, cache_key: &str) -> Arc<Guard> {
        let mut state = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = state.guards.get(cache_key).and_then(Weak::upgrade) {
            return existing;
        }
        state.guards.retain(|_, weak| weak.upgrade().is_some());
        let guard = Arc::new(Guard {
            lock: AsyncMutex::new(()),
            generation: AtomicU64::new(0_u64),
        });
        state
            .guards
            .insert(cache_key.to_owned(), Arc::downgrade(&guard));
        guard
    }

    fn store(&self, cache_key: &str, fingerprint: &str, token: &ExchangedToken) {
        let Some(expires_at) = Instant::now().checked_add(cache_ttl(token.expires_in)) else {
            return;
        };
        let mut state = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if state.entries.len() >= MAX_SIZE && !state.entries.contains_key(cache_key) {
            let oldest = state
                .entries
                .iter()
                .min_by_key(|(_key, entry)| entry.expires_at)
                .map(|(key, _entry)| key.clone());
            if let Some(key) = oldest {
                state.entries.remove(&key);
            }
        }
        state.entries.insert(
            cache_key.to_owned(),
            Entry {
                fingerprint: fingerprint.to_owned(),
                access_token: token.access_token.clone(),
                expires_at,
            },
        );
    }
}

fn cache_ttl(expires_in: Option<i64>) -> Duration {
    let lifetime = expires_in.unwrap_or(DEFAULT_TTL_SECONDS);
    let seconds = lifetime
        .saturating_sub(EXPIRY_BUFFER_SECONDS)
        .max(MIN_TTL_SECONDS);
    Duration::from_secs(seconds.unsigned_abs())
}
