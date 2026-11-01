mod tests;

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use secrecy::SecretString;

pub const BYOK_CREDENTIAL_CACHE_TTL_SECS: u64 = 60;
pub const BYOK_CREDENTIAL_CACHE_MAX_SIZE: usize = 4096;

pub const CACHE_KEY_PREFIX: &str = "mcp_byok_credential";

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Debug, Clone)]
pub struct CachedByokCredential {
    pub credential: Option<SecretString>,
}

type CacheKey = (String, String);

pub struct ByokCredentialCache {
    pub max_size: usize,
    pub ttl: Duration,
    pub clock: Box<dyn Clock>,
    pub entries: Mutex<HashMap<CacheKey, (CachedByokCredential, Instant)>>,
}

impl ByokCredentialCache {
    // Evicting it first if it expired keeps a stale TTL from being read as a live hit.
    pub fn get_cached_byok_credential(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Option<CachedByokCredential> {
        let key = (user_id.to_owned(), server_id.to_owned());
        let now = self.clock.now();
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        match entries.get(&key) {
            Some((value, stored_at)) if now.saturating_duration_since(*stored_at) < self.ttl => {
                Some(value.clone())
            }
            Some(_) => {
                entries.remove(&key);
                None
            }
            None => None,
        }
    }

    // credential: None is cached too, so a known-absent credential skips a repeat lookup.
    pub fn cache_byok_credential(
        &self,
        user_id: &str,
        server_id: &str,
        credential: Option<SecretString>,
    ) {
        let key = (user_id.to_owned(), server_id.to_owned());
        let now = self.clock.now();
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if entries.len() >= self.max_size && !entries.contains_key(&key) {
            let oldest = entries
                .iter()
                .min_by_key(|(_, (_, stored_at))| *stored_at)
                .map(|(oldest_key, _)| oldest_key.clone());
            if let Some(oldest_key) = oldest {
                entries.remove(&oldest_key);
            }
        }
        entries.insert(key, (CachedByokCredential { credential }, now));
    }
}
