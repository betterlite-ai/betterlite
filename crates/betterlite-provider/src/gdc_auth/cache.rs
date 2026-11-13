use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use tokio::sync::{Semaphore, SemaphorePermit};

pub const REFRESH_THRESHOLD_SECONDS: i64 = 225;

#[rustfmt::skip]
#[derive(Debug, Error)]
pub enum CacheError { #[error("the GDC Gemini token cache lease is closed")] Closed }
#[rustfmt::skip]
#[derive(Debug, Clone)]
pub enum CredentialIdentity { ApiKey(SecretString), SourceHandle(String) }
#[rustfmt::skip]
#[derive(Debug, Clone)]
pub struct CacheKey { pub audience: String, pub identity: CredentialIdentity }

#[rustfmt::skip]
fn identity_order(identity: &CredentialIdentity) -> (u8, &str) {
    match identity {
        CredentialIdentity::ApiKey(key) => (0, key.expose_secret()),
        CredentialIdentity::SourceHandle(handle) => (1, handle),
    }
}

#[rustfmt::skip]
impl PartialEq for CacheKey { fn eq(&self, other: &Self) -> bool { self.cmp(other).is_eq() } }
impl Eq for CacheKey {}
#[rustfmt::skip]
impl PartialOrd for CacheKey { fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) } }
#[rustfmt::skip]
impl Ord for CacheKey {
    fn cmp(&self, other: &Self) -> Ordering { self.audience.cmp(&other.audience).then_with(|| identity_order(&self.identity).cmp(&identity_order(&other.identity))) }
}

#[rustfmt::skip]
#[derive(Default)]
struct BoundHandle { token: Option<SecretString>, expires_at: Option<i64> }
#[rustfmt::skip]
#[derive(Debug)]
pub struct HandleState { pub token: Option<SecretString>, pub expires_at: Option<i64> }

pub struct TokenCache {
    entries: Mutex<BTreeMap<CacheKey, BoundHandle>>,
    lease: Semaphore,
}

#[rustfmt::skip]
impl TokenCache {
    pub async fn acquire(&self) -> Result<SemaphorePermit<'_>, CacheError> {
        self.lease
            .acquire()
            .await
            .map_err(|_error| CacheError::Closed)
    }
    // The bound handle enters the cache before its first refresh and stays there when that refresh fails
    pub fn state(&self, key: &CacheKey) -> HandleState {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let handle = entries.entry(key.clone()).or_default();
        let state = HandleState { token: handle.token.clone(), expires_at: handle.expires_at };
        drop(entries);
        state
    }
    pub fn store(&self, key: &CacheKey, token: SecretString, expires_at: Option<i64>) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(handle) = entries.get_mut(key) {
            handle.token = Some(token);
            handle.expires_at = expires_at;
        }
    }
    #[must_use]
    pub fn cache_keys(&self) -> Vec<CacheKey> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let keys = entries.keys().cloned().collect();
        drop(entries);
        keys
    }
}

#[rustfmt::skip]
impl Default for TokenCache {
    fn default() -> Self { Self { entries: Mutex::new(BTreeMap::new()), lease: Semaphore::const_new(1) } }
}

// The SDK considers a token stale 225 seconds before its expiry, and a truthy token is one that exists
#[must_use]
#[rustfmt::skip]
pub fn is_valid(token: Option<&SecretString>, expires_at: Option<i64>, now: i64) -> bool {
    let Some(present) = token else {
        return false;
    };
    let truthy = !present.expose_secret().is_empty();
    truthy && expires_at.is_none_or(|expiry| now < expiry.saturating_sub(REFRESH_THRESHOLD_SECONDS))
}
