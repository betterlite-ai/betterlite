#![cfg(test)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{
    BYOK_CREDENTIAL_CACHE_MAX_SIZE, BYOK_CREDENTIAL_CACHE_TTL_SECS, ByokCredentialCache,
    CachedByokCredential, Clock, SystemClock,
};

#[derive(Clone)]
struct FixedClock(Arc<Mutex<Instant>>);

impl FixedClock {
    fn new(now: Instant) -> Self {
        Self(Arc::new(Mutex::new(now)))
    }

    fn advance(&self, duration: Duration) {
        let mut now = self.0.lock().unwrap();
        *now = now
            .checked_add(duration)
            .expect("fake clock does not overflow");
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}

fn cache_with_limits(max_size: usize, ttl: Duration) -> ByokCredentialCache {
    ByokCredentialCache {
        max_size,
        ttl,
        clock: Box::new(SystemClock),
        entries: Mutex::new(HashMap::new()),
    }
}

fn credential_text(cached: Option<CachedByokCredential>) -> Option<String> {
    let present = cached?;
    present
        .credential
        .map(|secret| secret.expose_secret().to_owned())
}

#[rstest]
fn get_missing_returns_none() {
    let cache = cache_with_limits(
        BYOK_CREDENTIAL_CACHE_MAX_SIZE,
        Duration::from_secs(BYOK_CREDENTIAL_CACHE_TTL_SECS),
    );
    assert!(
        cache
            .get_cached_byok_credential("user-1", "server-1")
            .is_none()
    );
}

// The cache is typed to CachedByokCredential, so the Python isinstance guard has nothing to catch here.
#[rstest]
fn get_cached_type_check() {
    let cache = cache_with_limits(8, Duration::from_secs(60));
    cache.cache_byok_credential(
        "user-1",
        "server-1",
        Some(SecretString::from("api-key".to_owned())),
    );
    assert_eq!(
        credential_text(cache.get_cached_byok_credential("user-1", "server-1")),
        Some("api-key".to_owned())
    );

    cache.cache_byok_credential("user-2", "server-1", None);
    assert_eq!(
        credential_text(cache.get_cached_byok_credential("user-2", "server-1")),
        None
    );
    assert!(
        cache
            .get_cached_byok_credential("user-2", "server-1")
            .is_some()
    );
}

// A stored instant at "now" leaves the window at exactly zero, so a zero ttl must still read as expired.
#[rstest]
fn cache_treats_a_zero_ttl_window_as_already_stale() {
    let clock = FixedClock::new(Instant::now());
    let cache = ByokCredentialCache {
        max_size: 8,
        ttl: Duration::ZERO,
        clock: Box::new(clock),
        entries: Mutex::new(HashMap::new()),
    };
    cache.cache_byok_credential(
        "user-1",
        "server-1",
        Some(SecretString::from("api-key".to_owned())),
    );
    assert!(
        cache
            .get_cached_byok_credential("user-1", "server-1")
            .is_none()
    );
}

#[rstest]
fn cache_expires_once_the_clock_advances_past_the_ttl() {
    let clock = FixedClock::new(Instant::now());
    let cache = ByokCredentialCache {
        max_size: 8,
        ttl: Duration::from_secs(60),
        clock: Box::new(clock.clone()),
        entries: Mutex::new(HashMap::new()),
    };
    cache.cache_byok_credential(
        "user-1",
        "server-1",
        Some(SecretString::from("api-key".to_owned())),
    );
    assert_eq!(
        credential_text(cache.get_cached_byok_credential("user-1", "server-1")),
        Some("api-key".to_owned())
    );

    clock.advance(Duration::from_secs(61));
    assert!(
        cache
            .get_cached_byok_credential("user-1", "server-1")
            .is_none()
    );
}

#[rstest]
fn cache_keeps_entries_below_capacity() {
    let cache = cache_with_limits(2, Duration::from_secs(60));
    cache.cache_byok_credential(
        "user-1",
        "server-1",
        Some(SecretString::from("first".to_owned())),
    );
    cache.cache_byok_credential(
        "user-2",
        "server-1",
        Some(SecretString::from("second".to_owned())),
    );
    assert_eq!(
        credential_text(cache.get_cached_byok_credential("user-1", "server-1")),
        Some("first".to_owned())
    );
    assert_eq!(
        credential_text(cache.get_cached_byok_credential("user-2", "server-1")),
        Some("second".to_owned())
    );
}

#[rstest]
fn cache_evicts_the_oldest_entry_once_full() {
    let cache = cache_with_limits(1, Duration::from_secs(60));
    cache.cache_byok_credential(
        "user-1",
        "server-1",
        Some(SecretString::from("first".to_owned())),
    );
    cache.cache_byok_credential(
        "user-2",
        "server-1",
        Some(SecretString::from("second".to_owned())),
    );
    assert!(
        cache
            .get_cached_byok_credential("user-1", "server-1")
            .is_none()
    );
    assert_eq!(
        credential_text(cache.get_cached_byok_credential("user-2", "server-1")),
        Some("second".to_owned())
    );
}
