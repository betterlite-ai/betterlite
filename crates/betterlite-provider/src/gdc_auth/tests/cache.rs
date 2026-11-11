use std::cmp::Ordering;

use secrecy::{ExposeSecret as _, SecretString};

use crate::gdc_auth::cache::{
    CacheKey, CredentialIdentity, REFRESH_THRESHOLD_SECONDS, TokenCache, is_valid,
};

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn key(audience: &str, identity: CredentialIdentity) -> CacheKey {
    CacheKey {
        audience: audience.to_owned(),
        identity,
    }
}

#[test]
fn a_missing_token_is_never_valid() {
    assert!(!is_valid(None, None, 1_000));
}

#[test]
fn an_empty_token_is_never_valid() {
    let empty = secret("");
    assert!(!is_valid(Some(&empty), None, 1_000));
}

#[test]
fn a_token_without_an_expiry_stays_valid() {
    let token = secret("token");
    assert!(is_valid(Some(&token), None, 1_000));
}

#[test]
fn the_sdk_marks_a_token_expired_225_seconds_before_its_expiry() {
    let token = secret("token");
    let expiry: i64 = 1_000;
    let threshold = expiry.saturating_sub(REFRESH_THRESHOLD_SECONDS);
    assert!(is_valid(
        Some(&token),
        Some(expiry),
        threshold.saturating_sub(1)
    ));
    assert!(!is_valid(Some(&token), Some(expiry), threshold));
    assert!(!is_valid(Some(&token), Some(expiry), expiry));
    assert!(!is_valid(
        Some(&token),
        Some(expiry),
        expiry.saturating_add(1)
    ));
}

#[test]
fn a_token_expired_in_the_past_is_invalid() {
    let token = secret("token");
    assert!(!is_valid(Some(&token), Some(0), 1_000));
}

#[test]
fn cache_keys_separate_audiences_identities_and_kinds() {
    let cache = TokenCache::default();
    let same_audience_first = key(
        "https://one.example",
        CredentialIdentity::ApiKey(secret("key-one")),
    );
    let same_audience_second = key(
        "https://one.example",
        CredentialIdentity::ApiKey(secret("key-two")),
    );
    let other_audience = key(
        "https://two.example",
        CredentialIdentity::ApiKey(secret("key-one")),
    );
    let source_handle = key(
        "https://one.example",
        CredentialIdentity::SourceHandle("key-one".to_owned()),
    );
    cache.state(&same_audience_first);
    cache.state(&same_audience_second);
    cache.state(&other_audience);
    cache.state(&source_handle);
    assert_eq!(
        cache.cache_keys(),
        vec![
            same_audience_first.clone(),
            same_audience_second,
            source_handle.clone(),
            other_audience,
        ]
    );
    assert_ne!(same_audience_first, source_handle);
}

#[test]
fn cache_keys_compare_by_audience_then_identity() {
    let first = key(
        "https://one.example",
        CredentialIdentity::ApiKey(secret("key-one")),
    );
    let same = key(
        "https://one.example",
        CredentialIdentity::ApiKey(secret("key-one")),
    );
    let later_identity = key(
        "https://one.example",
        CredentialIdentity::ApiKey(secret("key-two")),
    );
    let later_audience = key(
        "https://two.example",
        CredentialIdentity::ApiKey(secret("key-one")),
    );
    assert_eq!(first, same);
    assert_eq!(first.partial_cmp(&same), Some(Ordering::Equal));
    assert_eq!(first.cmp(&later_identity), Ordering::Less);
    assert_eq!(later_identity.cmp(&first), Ordering::Greater);
    assert_eq!(first.partial_cmp(&later_audience), Some(Ordering::Less));
    assert_eq!(later_audience.cmp(&later_identity), Ordering::Greater);
}

#[test]
fn a_bound_handle_is_reused_until_the_token_is_stored() {
    let cache = TokenCache::default();
    let key = key(
        "https://one.example",
        CredentialIdentity::ApiKey(secret("key")),
    );
    assert!(cache.state(&key).token.is_none());
    cache.store(&key, secret("token"), Some(2_000));
    let state = cache.state(&key);
    assert_eq!(
        state.token.as_ref().map(SecretString::expose_secret),
        Some("token")
    );
    assert_eq!(state.expires_at, Some(2_000));
    assert_eq!(cache.cache_keys().len(), 1);
}
