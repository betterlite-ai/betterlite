mod tests;

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

pub const BYOK_CREDENTIAL_CACHE_TTL_SECS: u64 = 60;
pub const BYOK_CREDENTIAL_CACHE_MAX_SIZE: usize = 4096;

const CACHE_KEY_PREFIX: &str = "mcp_byok_credential";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedByokCredential {
    pub credential: Option<String>,
}

#[must_use]
pub fn byok_credential_cache_key(user_id: &str, server_id: &str) -> String {
    let capacity = CACHE_KEY_PREFIX
        .len()
        .saturating_add(user_id.len())
        .saturating_add(server_id.len())
        .saturating_add(2);
    let mut key = String::with_capacity(capacity);
    key.push_str(CACHE_KEY_PREFIX);
    key.push(':');
    key.push_str(user_id);
    key.push(':');
    key.push_str(server_id);
    key
}

pub struct ByokCredentialCache {
    pub max_size: usize,
    pub ttl: Duration,
    pub entries: Mutex<HashMap<String, (CachedByokCredential, Instant)>>,
}

impl ByokCredentialCache {
    // Evicting it first if it expired keeps a stale TTL from being read as a live hit.
    pub fn get_cached_byok_credential(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Option<CachedByokCredential> {
        let key = byok_credential_cache_key(user_id, server_id);
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        match entries.get(&key) {
            Some((value, stored_at)) if stored_at.elapsed() < self.ttl => Some(value.clone()),
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
        credential: Option<String>,
    ) {
        let key = byok_credential_cache_key(user_id, server_id);
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
        entries.insert(key, (CachedByokCredential { credential }, Instant::now()));
    }
}
