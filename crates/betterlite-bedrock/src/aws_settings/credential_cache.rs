use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest, Sha256};

const MAX_ENTRIES: usize = 1000;
const LOCK_COUNT: usize = 64;

#[derive(Clone)]
pub struct CachedCredentials {
    pub access_key_id: SecretString,
    pub secret_access_key: SecretString,
    pub session_token: Option<SecretString>,
}

struct Entry {
    credentials: CachedCredentials,
    expires: Option<Instant>,
}

fn entries() -> &'static Mutex<HashMap<String, Entry>> {
    static ENTRIES: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    ENTRIES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn locks() -> &'static [Mutex<()>; LOCK_COUNT] {
    static LOCKS: OnceLock<[Mutex<()>; LOCK_COUNT]> = OnceLock::new();
    LOCKS.get_or_init(|| std::array::from_fn(|_| Mutex::new(())))
}

pub fn cache_key(parameters: &serde_json::Value) -> Result<String, serde_json::Error> {
    let canonical = serde_json::to_string(parameters)?;
    let digest = Sha256::digest(canonical.as_bytes());
    let mut key = String::with_capacity(digest.len().saturating_mul(2));
    for byte in digest {
        key.push_str(&format!("{byte:02x}"));
    }
    Ok(key)
}

pub fn get_or_fetch<E, F>(
    key: &str,
    ttl: Option<Duration>,
    fetch: F,
) -> Result<CachedCredentials, E>
where
    F: FnOnce() -> Result<CachedCredentials, E>,
{
    let stripe = key.bytes().fold(0_usize, |value, byte| {
        value.wrapping_mul(31).wrapping_add(usize::from(byte))
    }) % LOCK_COUNT;
    let _guard = locks()[stripe]
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(hit) = entries()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(key)
        .filter(|entry| {
            entry.expires.is_none_or(|expires| expires > Instant::now())
                && !entry.credentials.access_key_id.expose_secret().is_empty()
                && !entry
                    .credentials
                    .secret_access_key
                    .expose_secret()
                    .is_empty()
        })
    {
        return Ok(hit.credentials.clone());
    }
    let credentials = fetch()?;
    let expires = ttl.and_then(|duration| Instant::now().checked_add(duration));
    let mut cache = entries()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache.retain(|_, entry| {
        entry
            .expires
            .is_none_or(|expiration| expiration > Instant::now())
    });
    if cache.len() >= MAX_ENTRIES {
        if let Some(oldest) = cache.keys().next().cloned() {
            cache.remove(&oldest);
        }
    }
    cache.insert(
        key.to_owned(),
        Entry {
            credentials: credentials.clone(),
            expires,
        },
    );
    Ok(credentials)
}
