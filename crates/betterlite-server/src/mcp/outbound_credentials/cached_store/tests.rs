#![cfg(test)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::CachedOAuthTokenStore;
use crate::mcp::outbound_credentials::{
    DEFAULT_EXPIRY_SKEW, InMemoryTokenCacheBackend, InvalidatableOAuthTokenStore as _, OAuthToken,
    OAuthTokenStore, TokenCacheEntries, TokenStoreUnavailable, WallClock,
};

#[derive(Debug)]
struct ManualClock(Mutex<DateTime<Utc>>);

impl ManualClock {
    fn set(&self, now: DateTime<Utc>) {
        *self.0.lock().unwrap() = now;
    }
}

impl WallClock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

struct ScriptedStore {
    results: Mutex<VecDeque<Result<Option<OAuthToken>, TokenStoreUnavailable>>>,
    reads: AtomicUsize,
}

impl ScriptedStore {
    fn new(results: Vec<Result<Option<OAuthToken>, TokenStoreUnavailable>>) -> Self {
        Self {
            results: Mutex::new(VecDeque::from(results)),
            reads: AtomicUsize::new(0),
        }
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl OAuthTokenStore for ScriptedStore {
    async fn fetch(
        &self,
        _user_id: &str,
        _server_id: &str,
    ) -> Result<Option<OAuthToken>, TokenStoreUnavailable> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.results
            .lock()
            .unwrap()
            .pop_front()
            .expect("a scripted result per read")
    }
}

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap()
}

fn token(access_token: &str, expires_at: Option<DateTime<Utc>>) -> OAuthToken {
    OAuthToken {
        access_token: access_token.to_owned(),
        expires_at,
        refresh_token: None,
        scopes: Vec::new(),
        identity_binding_proof: None,
    }
}

fn store(
    inner: ScriptedStore,
    clock: Arc<ManualClock>,
    default_ttl: Duration,
) -> CachedOAuthTokenStore<ScriptedStore, InMemoryTokenCacheBackend> {
    let concrete_clock: Arc<ManualClock> = clock;
    let backend_concrete: Arc<ManualClock> = Arc::clone(&concrete_clock);
    let backend_clock: Arc<dyn WallClock> = backend_concrete;
    let cache_clock: Arc<dyn WallClock> = concrete_clock;
    CachedOAuthTokenStore {
        inner,
        default_ttl,
        expiry_skew: DEFAULT_EXPIRY_SKEW,
        backend: InMemoryTokenCacheBackend {
            max_size: 4096,
            clock: backend_clock,
            entries: TokenCacheEntries::default(),
        },
        clock: cache_clock,
    }
}

#[tokio::test]
async fn cached_store_serves_a_positive_token_from_the_cache() {
    let clock = Arc::new(ManualClock(Mutex::new(at(100))));
    let inner = ScriptedStore::new(vec![Ok(Some(token("access", None)))]);
    let cached = store(inner, clock, Duration::from_secs(300));
    assert_eq!(
        cached.fetch("user", "server").await,
        Ok(Some(token("access", None)))
    );
    assert_eq!(
        cached.fetch("user", "server").await,
        Ok(Some(token("access", None)))
    );
    assert_eq!(cached.inner.reads(), 1);
}

#[tokio::test]
async fn cached_store_never_caches_a_miss() {
    let clock = Arc::new(ManualClock(Mutex::new(at(100))));
    let inner = ScriptedStore::new(vec![Ok(None), Ok(None)]);
    let cached = store(inner, clock, Duration::from_secs(300));
    assert_eq!(cached.fetch("user", "server").await, Ok(None));
    assert_eq!(cached.fetch("user", "server").await, Ok(None));
    assert_eq!(cached.inner.reads(), 2);
}

#[tokio::test]
async fn cached_store_propagates_unavailable_without_caching() {
    let clock = Arc::new(ManualClock(Mutex::new(at(100))));
    let unavailable = TokenStoreUnavailable {
        detail: "Database not connected".to_owned(),
    };
    let inner = ScriptedStore::new(vec![
        Err(unavailable.clone()),
        Ok(Some(token("access", None))),
    ]);
    let cached = store(inner, clock, Duration::from_secs(300));
    assert_eq!(cached.fetch("user", "server").await, Err(unavailable));
    assert_eq!(
        cached.fetch("user", "server").await,
        Ok(Some(token("access", None)))
    );
    assert_eq!(cached.inner.reads(), 2);
}

#[tokio::test]
async fn cached_store_re_reads_after_the_expiry_skew() {
    let clock = Arc::new(ManualClock(Mutex::new(at(900))));
    let inner = ScriptedStore::new(vec![
        Ok(Some(token("first", Some(at(1_000))))),
        Ok(Some(token("second", Some(at(2_000))))),
    ]);
    let cached = store(inner, Arc::clone(&clock), Duration::from_secs(300));
    assert_eq!(
        cached.fetch("user", "server").await,
        Ok(Some(token("first", Some(at(1_000)))))
    );
    clock.set(at(939));
    assert_eq!(
        cached.fetch("user", "server").await,
        Ok(Some(token("first", Some(at(1_000)))))
    );
    clock.set(at(940));
    assert_eq!(
        cached.fetch("user", "server").await,
        Ok(Some(token("second", Some(at(2_000)))))
    );
    assert_eq!(cached.inner.reads(), 2);
}

#[tokio::test]
async fn cached_store_uses_the_default_ttl_without_an_expiry() {
    let clock = Arc::new(ManualClock(Mutex::new(at(100))));
    let inner = ScriptedStore::new(vec![
        Ok(Some(token("first", None))),
        Ok(Some(token("second", None))),
    ]);
    let cached = store(inner, Arc::clone(&clock), Duration::from_secs(300));
    assert_eq!(
        cached.fetch("user", "server").await,
        Ok(Some(token("first", None)))
    );
    clock.set(at(399));
    assert_eq!(
        cached.fetch("user", "server").await,
        Ok(Some(token("first", None)))
    );
    clock.set(at(401));
    assert_eq!(
        cached.fetch("user", "server").await,
        Ok(Some(token("second", None)))
    );
    assert_eq!(cached.inner.reads(), 2);
}

#[tokio::test]
async fn cached_store_clamps_an_expired_token_ttl_to_zero() {
    let clock = Arc::new(ManualClock(Mutex::new(at(1_000))));
    let inner = ScriptedStore::new(vec![
        Ok(Some(token("first", Some(at(900))))),
        Ok(Some(token("second", None))),
    ]);
    let cached = store(inner, clock, Duration::from_secs(300));
    assert_eq!(
        cached.fetch("user", "server").await,
        Ok(Some(token("first", Some(at(900)))))
    );
    assert_eq!(
        cached.fetch("user", "server").await,
        Ok(Some(token("second", None)))
    );
    assert_eq!(cached.inner.reads(), 2);
}

#[tokio::test]
async fn cached_store_invalidate_drops_the_cached_entry() {
    let clock = Arc::new(ManualClock(Mutex::new(at(100))));
    let inner = ScriptedStore::new(vec![
        Ok(Some(token("first", None))),
        Ok(Some(token("second", None))),
    ]);
    let cached = store(inner, clock, Duration::from_secs(300));
    assert_eq!(
        cached.fetch("user", "server").await,
        Ok(Some(token("first", None)))
    );
    cached.invalidate("user", "server").await;
    assert_eq!(
        cached.fetch("user", "server").await,
        Ok(Some(token("second", None)))
    );
    assert_eq!(cached.inner.reads(), 2);
}
