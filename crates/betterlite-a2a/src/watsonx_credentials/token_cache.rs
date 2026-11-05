use std::num::NonZeroUsize;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use lru::LruCache;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};

mod tests;

pub trait TokenStore: Send + Sync {
    fn lookup(&self, key: &str, now: Duration) -> Option<SecretString>;
    fn insert(&self, key: String, token: SecretString, expires_at: Duration);
    fn evict_expired(&self, now: Duration);
}

struct CachedToken {
    token: SecretString,
    expires_at: Duration,
}

pub struct BoundedTokenCache {
    entries: Mutex<LruCache<String, CachedToken>>,
}

impl BoundedTokenCache {
    #[must_use]
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self {
            entries: Mutex::new(LruCache::new(capacity)),
        }
    }
}

impl TokenStore for BoundedTokenCache {
    fn lookup(&self, key: &str, now: Duration) -> Option<SecretString> {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries
            .get(key)
            .filter(|cached| cached.expires_at > now)
            .map(|cached| SecretString::from(cached.token.expose_secret()))
    }

    fn insert(&self, key: String, token: SecretString, expires_at: Duration) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries.put(key, CachedToken { token, expires_at });
    }

    fn evict_expired(&self, now: Duration) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let stale = entries
            .iter()
            .filter(|(_, cached)| cached.expires_at <= now)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in stale {
            entries.pop(&key);
        }
    }
}

#[must_use]
pub fn token_cache_key(
    auth_mode: &str,
    cp4d_host: &str,
    username: Option<&str>,
    api_key: &SecretString,
) -> String {
    let digest = Sha256::new()
        .chain_update(auth_mode)
        .chain_update(":")
        .chain_update(cp4d_host)
        .chain_update(":")
        .chain_update(username.unwrap_or_default())
        .chain_update(":")
        .chain_update(api_key.expose_secret())
        .finalize();
    hex::encode(digest)
}
