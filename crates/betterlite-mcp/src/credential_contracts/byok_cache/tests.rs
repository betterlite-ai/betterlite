#![cfg(test)]

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use rstest::rstest;

use super::{
    BYOK_CREDENTIAL_CACHE_MAX_SIZE, BYOK_CREDENTIAL_CACHE_TTL_SECS, ByokCredentialCache,
    CachedByokCredential, byok_credential_cache_key,
};

fn cache_with_limits(max_size: usize, ttl: Duration) -> ByokCredentialCache {
    ByokCredentialCache {
        max_size,
        ttl,
        entries: Mutex::new(HashMap::new()),
    }
}

#[rstest]
fn cache_key_format() {
    assert_eq!(
        byok_credential_cache_key("user-1", "server-1"),
        "mcp_byok_credential:user-1:server-1"
    );
    assert_eq!(
        byok_credential_cache_key("user-1", "server-2"),
        "mcp_byok_credential:user-1:server-2"
    );
}

#[rstest]
fn get_missing_returns_none() {
    let cache = cache_with_limits(
        BYOK_CREDENTIAL_CACHE_MAX_SIZE,
        Duration::from_secs(BYOK_CREDENTIAL_CACHE_TTL_SECS),
    );
    assert_eq!(cache.get_cached_byok_credential("user-1", "server-1"), None);
}

// The cache is typed to CachedByokCredential, so the Python isinstance guard has nothing to catch here.
#[rstest]
fn get_cached_type_check() {
    let cache = cache_with_limits(8, Duration::from_secs(60));
    cache.cache_byok_credential("user-1", "server-1", Some("api-key".to_owned()));
    let cached = cache.get_cached_byok_credential("user-1", "server-1");
    assert_eq!(
        cached,
        Some(CachedByokCredential {
            credential: Some("api-key".to_owned())
        })
    );

    cache.cache_byok_credential("user-2", "server-1", None);
    assert_eq!(
        cache.get_cached_byok_credential("user-2", "server-1"),
        Some(CachedByokCredential { credential: None })
    );
}

#[rstest]
fn cache_evicts_the_oldest_entry_once_full() {
    let cache = cache_with_limits(1, Duration::from_secs(60));
    cache.cache_byok_credential("user-1", "server-1", Some("first".to_owned()));
    cache.cache_byok_credential("user-2", "server-1", Some("second".to_owned()));
    assert_eq!(cache.get_cached_byok_credential("user-1", "server-1"), None);
    assert_eq!(
        cache.get_cached_byok_credential("user-2", "server-1"),
        Some(CachedByokCredential {
            credential: Some("second".to_owned())
        })
    );
}
