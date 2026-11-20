#![cfg(test)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::DateTime;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{CacheState, SsoAssertionCache};
use crate::credential_contracts::byok_cache::Clock;
use crate::sso_assertions::assertion::SsoIdentityAssertion;

#[derive(Clone)]
struct FixedClock(Arc<Mutex<Instant>>);

impl FixedClock {
    fn advance(&self, by: Duration) {
        let mut now = self.0.lock().unwrap();
        *now = now.checked_add(by).unwrap();
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}

fn cache_with(ttl: Duration, max_size: usize) -> (SsoAssertionCache, FixedClock) {
    let clock = FixedClock(Arc::new(Mutex::new(Instant::now())));
    let cache = SsoAssertionCache {
        ttl,
        max_size,
        clock: Box::new(clock.clone()),
        state: Mutex::new(CacheState::default()),
    };
    (cache, clock)
}

fn assertion(token: &str) -> SsoIdentityAssertion {
    SsoIdentityAssertion {
        id_token: SecretString::from(token.to_owned()),
        refresh_token: None,
        issuer: None,
        expires_at: None,
    }
}

fn token_of(assertion: Option<SsoIdentityAssertion>) -> Option<String> {
    assertion.map(|found| found.id_token.expose_secret().to_owned())
}

#[rstest]
fn get_returns_none_for_an_unknown_user() {
    let (cache, _clock) = cache_with(Duration::from_secs(60), 8);
    assert!(cache.get("alice").is_none());
}

#[rstest]
fn get_returns_a_live_assertion() {
    let (cache, _clock) = cache_with(Duration::from_secs(60), 8);
    cache.set_if_unchanged("alice", &assertion("first-token"), 0);
    assert_eq!(token_of(cache.get("alice")).as_deref(), Some("first-token"));
}

#[rstest]
fn get_drops_an_entry_at_its_expiry() {
    let (cache, clock) = cache_with(Duration::from_secs(60), 8);
    cache.set_if_unchanged("alice", &assertion("first-token"), 0);
    clock.advance(Duration::from_secs(60));
    assert!(cache.get("alice").is_none());
    assert!(cache.state.lock().unwrap().entries.is_empty());
}

#[rstest]
fn a_zero_ttl_expires_an_entry_at_once() {
    let (cache, _clock) = cache_with(Duration::ZERO, 8);
    cache.set_if_unchanged("alice", &assertion("first-token"), 0);
    assert!(cache.get("alice").is_none());
}

#[rstest]
fn rewriting_a_live_key_keeps_its_first_expiry() {
    let (cache, clock) = cache_with(Duration::from_secs(60), 8);
    cache.set_if_unchanged("alice", &assertion("first-token"), 0);
    clock.advance(Duration::from_secs(30));
    cache.set_if_unchanged("alice", &assertion("second-token"), cache.epoch());
    assert_eq!(
        token_of(cache.get("alice")).as_deref(),
        Some("second-token")
    );
    clock.advance(Duration::from_secs(31));
    assert!(cache.get("alice").is_none());
}

#[rstest]
fn rewriting_an_expired_key_starts_a_new_lifetime() {
    let (cache, clock) = cache_with(Duration::from_secs(60), 8);
    cache.set_if_unchanged("alice", &assertion("first-token"), 0);
    clock.advance(Duration::from_secs(61));
    cache.set_if_unchanged("alice", &assertion("second-token"), cache.epoch());
    clock.advance(Duration::from_secs(1));
    assert_eq!(
        token_of(cache.get("alice")).as_deref(),
        Some("second-token")
    );
}

#[rstest]
#[case::same_user("alice", None)]
#[case::other_user("bravo", Some("first-token"))]
fn set_if_unchanged_skips_a_stale_epoch(
    #[case] invalidated_user: &str,
    #[case] expected: Option<&str>,
) {
    let (cache, _clock) = cache_with(Duration::from_secs(60), 8);
    cache.set_if_unchanged("alice", &assertion("first-token"), 0);
    cache.invalidate(invalidated_user);
    cache.set_if_unchanged("alice", &assertion("second-token"), 0);
    assert_eq!(token_of(cache.get("alice")).as_deref(), expected);
}

#[rstest]
fn an_invalidate_bumps_the_epoch_even_for_an_absent_key() {
    let (cache, _clock) = cache_with(Duration::from_secs(60), 8);
    assert_eq!(cache.epoch(), 0);
    cache.invalidate("alice");
    assert_eq!(cache.epoch(), 1);
    cache.invalidate("bravo");
    assert_eq!(cache.epoch(), 2);
}

#[rstest]
fn invalidate_removes_only_that_users_entry() {
    let (cache, _clock) = cache_with(Duration::from_secs(60), 8);
    cache.set_if_unchanged("alice", &assertion("first-token"), 0);
    cache.set_if_unchanged("bravo", &assertion("second-token"), 0);
    cache.invalidate("alice");
    assert!(cache.get("alice").is_none());
    assert_eq!(
        token_of(cache.get("bravo")).as_deref(),
        Some("second-token")
    );
}

#[rstest]
fn max_size_zero_stores_nothing() {
    let (cache, _clock) = cache_with(Duration::from_secs(60), 0);
    cache.set_if_unchanged("alice", &assertion("first-token"), 0);
    assert!(cache.get("alice").is_none());
    assert!(cache.state.lock().unwrap().entries.is_empty());
}

#[rstest]
fn a_write_drops_every_expired_entry() {
    let (cache, clock) = cache_with(Duration::from_secs(60), 3);
    cache.set_if_unchanged("alice", &assertion("first-token"), 0);
    cache.set_if_unchanged("bravo", &assertion("second-token"), 0);
    clock.advance(Duration::from_secs(61));
    cache.set_if_unchanged("charlie", &assertion("third-token"), cache.epoch());
    assert_eq!(cache.state.lock().unwrap().entries.len(), 1);
    assert!(cache.get("alice").is_none());
    assert!(cache.get("bravo").is_none());
    assert_eq!(
        token_of(cache.get("charlie")).as_deref(),
        Some("third-token")
    );
}

#[rstest]
fn a_full_cache_evicts_the_earliest_expiry() {
    let (cache, clock) = cache_with(Duration::from_secs(60), 2);
    cache.set_if_unchanged("alice", &assertion("first-token"), 0);
    clock.advance(Duration::from_secs(1));
    cache.set_if_unchanged("bravo", &assertion("second-token"), cache.epoch());
    clock.advance(Duration::from_secs(1));
    cache.set_if_unchanged("charlie", &assertion("third-token"), cache.epoch());
    assert!(cache.get("alice").is_none());
    assert_eq!(
        token_of(cache.get("bravo")).as_deref(),
        Some("second-token")
    );
    assert_eq!(
        token_of(cache.get("charlie")).as_deref(),
        Some("third-token")
    );
}

#[rstest]
fn an_expiry_tie_evicts_the_first_key() {
    let (cache, _clock) = cache_with(Duration::from_secs(60), 2);
    cache.set_if_unchanged("bravo", &assertion("second-token"), 0);
    cache.set_if_unchanged("alice", &assertion("first-token"), 0);
    cache.set_if_unchanged("charlie", &assertion("third-token"), 0);
    assert!(cache.get("alice").is_none());
    assert_eq!(
        token_of(cache.get("bravo")).as_deref(),
        Some("second-token")
    );
    assert_eq!(
        token_of(cache.get("charlie")).as_deref(),
        Some("third-token")
    );
}

#[rstest]
fn the_cache_never_judges_the_assertions_own_expiry() {
    let (cache, _clock) = cache_with(Duration::from_secs(60), 8);
    let mut stale = assertion("first-token");
    stale.expires_at = DateTime::from_timestamp(0, 0);
    cache.set_if_unchanged("alice", &stale, 0);
    assert_eq!(token_of(cache.get("alice")).as_deref(), Some("first-token"));
}
