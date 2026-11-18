#![cfg(test)]

use super::{
    CacheDeleteFuture, GLOBAL_PROXY_SPEND_CACHE_KEY, InvalidatedCache, ResetCache,
    invalidate_caches,
};
use crate::budget_reset_policy::tests::logging::{Logs, subscriber};
use std::sync::{Arc, Mutex, PoisonError};
use tracing::instrument::WithSubscriber as _;

#[derive(Debug, thiserror::Error)]
#[error("the cache refused the entries")]
struct CacheFault;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failures {
    None,
    SpendCounter,
    UserApiKeyCache,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheCall {
    sink: InvalidatedCache,
    keys: Vec<String>,
    failed: bool,
}

#[derive(Clone)]
struct FakeCache {
    calls: Arc<Mutex<Vec<CacheCall>>>,
    failures: Failures,
}

impl FakeCache {
    fn recorded(&self) -> Vec<CacheCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn fake_cache(failures: Failures) -> FakeCache {
    FakeCache {
        calls: Arc::new(Mutex::new(Vec::new())),
        failures,
    }
}

impl ResetCache for FakeCache {
    type Failure = CacheFault;

    fn delete_spend_counters<'cache>(
        &'cache self,
        keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, CacheFault> {
        let failed = matches!(self.failures, Failures::SpendCounter);
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CacheCall {
                sink: InvalidatedCache::SpendCounter,
                keys: keys.to_vec(),
                failed,
            });
        Box::pin(async move { if failed { Err(CacheFault) } else { Ok(()) } })
    }

    fn delete_user_api_keys<'cache>(
        &'cache self,
        keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, CacheFault> {
        let failed = matches!(self.failures, Failures::UserApiKeyCache);
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CacheCall {
                sink: InvalidatedCache::UserApiKeyCache,
                keys: keys.to_vec(),
                failed,
            });
        Box::pin(async move { if failed { Err(CacheFault) } else { Ok(()) } })
    }
}

#[test]
fn cache_invalidation_names_and_global_key_match_litellms_literals() {
    assert_eq!(InvalidatedCache::SpendCounter.as_str(), "spend counter");
    assert_eq!(
        InvalidatedCache::UserApiKeyCache.as_str(),
        "user_api_key_cache"
    );
    assert_eq!(GLOBAL_PROXY_SPEND_CACHE_KEY, "default_user_id:spend");
}

#[tokio::test]
async fn cache_invalidation_drops_both_sinks() {
    let cache = fake_cache(Failures::None);
    invalidate_caches(
        &cache,
        &["spend:key:k1".to_owned()],
        &[GLOBAL_PROXY_SPEND_CACHE_KEY.to_owned()],
    )
    .await;
    let calls = cache.recorded();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].sink, InvalidatedCache::SpendCounter);
    assert_eq!(calls[0].keys, ["spend:key:k1"]);
    assert!(!calls[0].failed);
    assert_eq!(calls[1].sink, InvalidatedCache::UserApiKeyCache);
    assert_eq!(calls[1].keys, [GLOBAL_PROXY_SPEND_CACHE_KEY]);
    assert!(!calls[1].failed);
}

#[tokio::test]
async fn cache_invalidation_keeps_the_user_cache_when_the_counter_fails() {
    let cache = fake_cache(Failures::SpendCounter);
    invalidate_caches(&cache, &["spend:key:k1".to_owned()], &["k1".to_owned()]).await;
    let calls = cache.recorded();
    assert_eq!(calls.len(), 2);
    assert!(calls[0].failed);
    assert_eq!(calls[1].sink, InvalidatedCache::UserApiKeyCache);
    assert!(!calls[1].failed);
}

#[tokio::test]
async fn cache_invalidation_keeps_the_counter_when_the_user_cache_fails() {
    let cache = fake_cache(Failures::UserApiKeyCache);
    invalidate_caches(&cache, &["spend:key:k1".to_owned()], &["k1".to_owned()]).await;
    let calls = cache.recorded();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].sink, InvalidatedCache::SpendCounter);
    assert!(!calls[0].failed);
    assert!(calls[1].failed);
}

#[tokio::test]
async fn cache_invalidation_skips_an_empty_batch() {
    let cache = fake_cache(Failures::SpendCounter);
    invalidate_caches(&cache, &[], &[]).await;
    assert!(cache.recorded().is_empty());
}

#[tokio::test]
async fn cache_invalidation_logs_a_warning_when_a_sink_fails() {
    let logs = Logs::default();
    let cache = fake_cache(Failures::SpendCounter);
    invalidate_caches(&cache, &["spend:key:k1".to_owned()], &[])
        .with_subscriber(subscriber(logs.clone()))
        .await;
    let text = logs.text();
    assert!(text.contains("Failed to invalidate cache entries"));
    assert!(text.contains("spend counter"));
    assert!(text.contains("the cache refused the entries"));
}
