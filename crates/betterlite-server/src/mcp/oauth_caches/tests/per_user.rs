use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;

use super::base_server;
use crate::mcp::oauth_caches::{MCPPerUserTokenCache, compute_per_user_token_ttl};
use crate::mcp::outbound_credentials::PER_USER_TOKEN_KEY_PREFIX;
use crate::mcp::outbound_credentials::cache_codec::OAuthTokenCacheCodec;
use crate::mcp::outbound_credentials::dual_cache::{AsyncCache, DualCacheTokenCacheBackend};

#[derive(Debug, thiserror::Error)]
#[error("the fake backend failed")]
struct FakeError;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Failure {
    Get,
    Set,
    Delete,
}

struct FakeBackend {
    entries: Mutex<BTreeMap<String, String>>,
    failures: BTreeSet<Failure>,
    set_calls: Mutex<Vec<(String, i64)>>,
    delete_calls: Mutex<Vec<String>>,
}

impl FakeBackend {
    fn new() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
            failures: BTreeSet::new(),
            set_calls: Mutex::new(Vec::new()),
            delete_calls: Mutex::new(Vec::new()),
        }
    }

    fn with_entry(key: &str, value: &str) -> Self {
        let backend = Self::new();
        backend.hold(key, value);
        backend
    }

    fn failing(failure: Failure) -> Self {
        Self {
            failures: BTreeSet::from([failure]),
            ..Self::new()
        }
    }

    fn refuse(&self, failure: Failure) -> Result<(), FakeError> {
        if self.failures.contains(&failure) {
            return Err(FakeError);
        }
        Ok(())
    }

    fn value(&self, key: &str) -> Option<String> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    fn store(&self, key: &str, value: &str, ttl: Option<Duration>) {
        self.set_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), ttl_seconds(ttl)));
        self.hold(key, value);
    }

    fn evict(&self, key: &str) {
        self.delete_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.to_owned());
        self.forget(key);
    }

    fn hold(&self, key: &str, value: &str) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        drop(entries.insert(key.to_owned(), value.to_owned()));
    }

    fn forget(&self, key: &str) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        drop(entries.remove(key));
    }
}

#[async_trait]
impl AsyncCache for FakeBackend {
    type Error = FakeError;

    async fn async_get_cache(&self, key: &str) -> Result<Option<CacheValue>, FakeError> {
        self.refuse(Failure::Get)?;
        Ok(self.value(key).map(CacheValue::Text))
    }

    async fn async_set_cache(
        &self,
        key: &str,
        value: &str,
        ttl: Option<Duration>,
    ) -> Result<(), FakeError> {
        self.refuse(Failure::Set)?;
        self.store(key, value, ttl);
        Ok(())
    }

    async fn async_delete_cache(&self, key: &str) -> Result<(), FakeError> {
        self.refuse(Failure::Delete)?;
        self.evict(key);
        Ok(())
    }
}

fn ttl_seconds(ttl: Option<Duration>) -> i64 {
    ttl.map_or(0, |lifetime| {
        i64::try_from(lifetime.as_secs()).unwrap_or(i64::MAX)
    })
}

fn codec() -> OAuthTokenCacheCodec {
    OAuthTokenCacheCodec {
        encrypt: Box::new(|plain: &str| format!("sealed:{plain}")),
        decrypt: Box::new(|blob: &str| blob.strip_prefix("sealed:").map(str::to_owned)),
    }
}

fn per_user_cache(
    backend: FakeBackend,
) -> MCPPerUserTokenCache<DualCacheTokenCacheBackend<FakeBackend>> {
    MCPPerUserTokenCache {
        backend: DualCacheTokenCacheBackend {
            cache: backend,
            codec: codec(),
            key_prefix: PER_USER_TOKEN_KEY_PREFIX,
        },
    }
}

#[test]
fn per_user_cache_key_matches_litellm() {
    let key = format!("{PER_USER_TOKEN_KEY_PREFIX}user-1:srv-1");
    assert_eq!(key, "mcp:per_user_token:user-1:srv-1");
}

#[tokio::test]
async fn per_user_set_stores_an_encrypted_value_with_the_ttl_and_proof() {
    let cache = per_user_cache(FakeBackend::new());
    cache
        .set("user-1", "srv-1", "tok", 120, Some("proof"))
        .await;
    let stored = cache
        .backend
        .cache
        .value("mcp:per_user_token:user-1:srv-1")
        .expect("the value is stored");
    assert_ne!(stored, "tok");
    assert_ne!(stored, "sealed:tok");
    assert!(stored.starts_with("sealed:litellm-bound-oauth-v1:"));
    assert_eq!(
        cache
            .backend
            .cache
            .set_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_slice(),
        &[("mcp:per_user_token:user-1:srv-1".to_owned(), 120)]
    );
    let token = cache
        .get_token("user-1", "srv-1")
        .await
        .expect("the token decodes");
    assert_eq!(token.access_token, "tok");
    assert_eq!(token.identity_binding_proof.as_deref(), Some("proof"));
    assert_eq!(cache.get("user-1", "srv-1").await, Some("tok".to_owned()));
}

#[tokio::test]
async fn per_user_set_stores_a_bearer_only_value_without_a_proof() {
    let cache = per_user_cache(FakeBackend::new());
    cache.set("user-1", "srv-1", "tok", 60, None).await;
    let stored = cache
        .backend
        .cache
        .value("mcp:per_user_token:user-1:srv-1")
        .expect("the value is stored");
    assert_eq!(stored, "sealed:tok");
    assert_eq!(cache.get("user-1", "srv-1").await, Some("tok".to_owned()));
}

#[tokio::test]
async fn per_user_set_skips_a_ttl_that_is_not_positive() {
    let cache = per_user_cache(FakeBackend::new());
    cache.set("user-1", "srv-1", "tok", 0, None).await;
    cache.set("user-1", "srv-1", "tok", -5, None).await;
    assert!(
        cache
            .backend
            .cache
            .set_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
    assert_eq!(cache.get("user-1", "srv-1").await, None);
}

#[tokio::test]
async fn per_user_set_swallows_a_backend_failure() {
    let cache = per_user_cache(FakeBackend::failing(Failure::Set));
    cache.set("user-1", "srv-1", "tok", 60, None).await;
    assert_eq!(cache.get("user-1", "srv-1").await, None);
}

#[tokio::test]
async fn per_user_get_token_reads_an_undecryptable_value_as_a_miss() {
    let cache = per_user_cache(FakeBackend::with_entry(
        "mcp:per_user_token:user-1:srv-1",
        "not-a-sealed-value",
    ));
    assert_eq!(cache.get_token("user-1", "srv-1").await, None);
    assert_eq!(cache.get("user-1", "srv-1").await, None);
}

#[tokio::test]
async fn per_user_get_token_swallows_a_backend_failure() {
    let cache = per_user_cache(FakeBackend::failing(Failure::Get));
    assert_eq!(cache.get_token("user-1", "srv-1").await, None);
}

#[tokio::test]
async fn per_user_delete_evicts_the_key_and_swallows_failures() {
    let cache = per_user_cache(FakeBackend::new());
    cache.set("user-1", "srv-1", "tok", 60, None).await;
    cache.delete("user-1", "srv-1").await;
    assert_eq!(cache.get("user-1", "srv-1").await, None);
    assert_eq!(
        cache
            .backend
            .cache
            .delete_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_slice(),
        &["mcp:per_user_token:user-1:srv-1".to_owned()]
    );
    let failing = per_user_cache(FakeBackend::failing(Failure::Delete));
    failing.delete("user-1", "srv-1").await;
}

#[test]
fn compute_per_user_token_ttl_uses_the_configured_value_when_shorter() {
    let mut configured = base_server();
    configured.fields.token_storage_ttl_seconds = Some(7200);
    assert_eq!(compute_per_user_token_ttl(&configured, Some(86_400)), 7200);
    assert_eq!(compute_per_user_token_ttl(&configured, None), 7200);
    let mut zero = base_server();
    zero.fields.token_storage_ttl_seconds = Some(0);
    assert_eq!(compute_per_user_token_ttl(&zero, None), 1);
}

#[test]
fn compute_per_user_token_ttl_is_bounded_by_the_token_lifetime() {
    let mut configured = base_server();
    configured.fields.token_storage_ttl_seconds = Some(7200);
    assert_eq!(compute_per_user_token_ttl(&configured, Some(1800)), 1740);
    let unconfigured = base_server();
    assert_eq!(compute_per_user_token_ttl(&unconfigured, Some(1800)), 1740);
    assert_eq!(compute_per_user_token_ttl(&unconfigured, Some(30)), 1);
    assert_eq!(compute_per_user_token_ttl(&unconfigured, Some(61)), 1);
    assert_eq!(compute_per_user_token_ttl(&unconfigured, Some(62)), 2);
}

#[test]
fn compute_per_user_token_ttl_falls_back_to_the_default() {
    assert_eq!(compute_per_user_token_ttl(&base_server(), None), 43_200);
}
