#![cfg(test)]

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};

use super::{BoundedTokenCache, TokenStore as _, token_cache_key};

fn cache(capacity: usize) -> BoundedTokenCache {
    BoundedTokenCache::new(NonZeroUsize::new(capacity).unwrap())
}

fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

fn token(cache: &BoundedTokenCache, key: &str, now: u64) -> Option<String> {
    cache
        .lookup(key, secs(now))
        .map(|found| found.expose_secret().to_owned())
}

#[test]
fn cache_key_hashes_mode_host_username_and_key_in_that_order() {
    let key = SecretString::from("secret");
    let expected = hex::encode(Sha256::digest("cp4d:https://h:bob:secret"));
    assert_eq!(
        token_cache_key("cp4d", "https://h", Some("bob"), &key),
        expected
    );
}

#[test]
fn cache_key_uses_an_empty_username_when_missing() {
    let key = SecretString::from("secret");
    let expected = hex::encode(Sha256::digest("ibm_cloud:https://h::secret"));
    assert_eq!(
        token_cache_key("ibm_cloud", "https://h", None, &key),
        expected
    );
}

#[rstest]
#[case::mode("ibm_cloud", "h", Some("u"), "k")]
#[case::host("cp4d", "other", Some("u"), "k")]
#[case::username("cp4d", "h", Some("other"), "k")]
#[case::api_key("cp4d", "h", Some("u"), "other")]
fn cache_key_changes_with_every_part(
    #[case] mode: &str,
    #[case] host: &str,
    #[case] username: Option<&str>,
    #[case] api_key: &str,
) {
    let base = token_cache_key("cp4d", "h", Some("u"), &SecretString::from("k"));
    assert_ne!(
        token_cache_key(mode, host, username, &SecretString::from(api_key)),
        base
    );
}

#[test]
fn cache_hit_needs_the_expiry_to_be_after_now() {
    let cache = cache(4);
    cache.insert("k".to_owned(), SecretString::from("t"), secs(100));
    assert_eq!(token(&cache, "k", 99).as_deref(), Some("t"));
    assert_eq!(token(&cache, "k", 100), None);
    assert_eq!(token(&cache, "k", 101), None);
}

#[test]
fn cache_evicts_every_entry_that_expired_and_keeps_the_rest() {
    let cache = cache(4);
    cache.insert("old".to_owned(), SecretString::from("a"), secs(50));
    cache.insert("edge".to_owned(), SecretString::from("b"), secs(60));
    cache.insert("live".to_owned(), SecretString::from("c"), secs(61));
    cache.evict_expired(secs(60));
    assert_eq!(token(&cache, "old", 0), None);
    assert_eq!(token(&cache, "edge", 0), None);
    assert_eq!(token(&cache, "live", 0).as_deref(), Some("c"));
}

#[test]
fn a_shared_arc_store_reaches_the_cache_behind_it() {
    let cache = cache(2);
    let shared = Arc::new(cache);
    shared.insert("k".to_owned(), SecretString::from("t"), secs(10));
    assert_eq!(
        shared
            .lookup("k", secs(9))
            .map(|token| token.expose_secret().to_owned())
            .as_deref(),
        Some("t")
    );
    shared.evict_expired(secs(10));
    assert!(shared.lookup("k", secs(0)).is_none());
}

#[test]
fn cache_is_bounded_and_drops_the_least_recently_used_entry() {
    let cache = cache(2);
    cache.insert("a".to_owned(), SecretString::from("1"), secs(1000));
    cache.insert("b".to_owned(), SecretString::from("2"), secs(1000));
    assert_eq!(token(&cache, "a", 0).as_deref(), Some("1"));
    cache.insert("c".to_owned(), SecretString::from("3"), secs(1000));
    assert_eq!(token(&cache, "b", 0), None);
    assert_eq!(token(&cache, "a", 0).as_deref(), Some("1"));
    assert_eq!(token(&cache, "c", 0).as_deref(), Some("3"));
}
