#![cfg(test)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::task::yield_now;

use super::{InProcessRefreshCoordinator, RefreshingTokenStore};
use crate::mcp::outbound_credentials::{
    DEFAULT_EXPIRY_SKEW, OAuthToken, OAuthTokenStore, RefreshCoordinator, RefreshStep, TokenKey,
    TokenRefresher, TokenStoreUnavailable, WallClock,
};

type Step<'call> = RefreshStep<'call, Option<OAuthToken>, TokenStoreUnavailable>;

#[derive(Debug)]
struct ManualClock(Mutex<DateTime<Utc>>);

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

struct ScriptedRefresher {
    results: Mutex<VecDeque<Result<Option<OAuthToken>, TokenStoreUnavailable>>>,
    calls: AtomicUsize,
}

impl ScriptedRefresher {
    fn new(results: Vec<Result<Option<OAuthToken>, TokenStoreUnavailable>>) -> Self {
        Self {
            results: Mutex::new(VecDeque::from(results)),
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl TokenRefresher for ScriptedRefresher {
    async fn refresh(
        &self,
        _user_id: &str,
        _server_id: &str,
        _token: &OAuthToken,
    ) -> Result<Option<OAuthToken>, TokenStoreUnavailable> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.results
            .lock()
            .unwrap()
            .pop_front()
            .expect("a scripted result per refresh")
    }
}

struct RereadOnly;

#[async_trait]
impl RefreshCoordinator for RereadOnly {
    type Output = Option<OAuthToken>;
    type Error = TokenStoreUnavailable;

    async fn run(
        &self,
        _key: TokenKey,
        _refresh: Step<'_>,
        reread: Step<'_>,
    ) -> Result<Option<OAuthToken>, TokenStoreUnavailable> {
        reread().await
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

fn key(user_id: &str, server_id: &str) -> TokenKey {
    TokenKey {
        user_id: user_id.to_owned(),
        server_id: server_id.to_owned(),
    }
}

fn clock_at(seconds: i64) -> Arc<ManualClock> {
    Arc::new(ManualClock(Mutex::new(at(seconds))))
}

fn refresh_step<'call>(calls: &'call AtomicUsize, value: &'call str) -> Step<'call> {
    Box::new(move || {
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            yield_now().await;
            Ok(Some(token(value, None)))
        })
    })
}

fn failing_step(calls: &AtomicUsize) -> Step<'_> {
    Box::new(move || {
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            yield_now().await;
            Err(TokenStoreUnavailable {
                detail: "the refresh failed".to_owned(),
            })
        })
    })
}

fn reread_step(calls: &AtomicUsize) -> Step<'_> {
    Box::new(move || {
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        })
    })
}

#[tokio::test]
async fn coordinator_runs_one_refresh_for_concurrent_callers() {
    let coordinator: InProcessRefreshCoordinator<Option<OAuthToken>, TokenStoreUnavailable> =
        InProcessRefreshCoordinator::default();
    let refreshes = AtomicUsize::new(0);
    let rereads = AtomicUsize::new(0);
    let (first, second, third) = tokio::join!(
        coordinator.run(
            key("user", "server"),
            refresh_step(&refreshes, "one"),
            reread_step(&rereads)
        ),
        coordinator.run(
            key("user", "server"),
            refresh_step(&refreshes, "one"),
            reread_step(&rereads)
        ),
        coordinator.run(
            key("user", "server"),
            refresh_step(&refreshes, "one"),
            reread_step(&rereads)
        ),
    );
    let expected = Ok(Some(token("one", None)));
    assert_eq!(first, expected);
    assert_eq!(second, expected);
    assert_eq!(third, expected);
    assert_eq!(refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(rereads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn coordinator_shares_a_failure_with_every_caller() {
    let coordinator: InProcessRefreshCoordinator<Option<OAuthToken>, TokenStoreUnavailable> =
        InProcessRefreshCoordinator::default();
    let refreshes = AtomicUsize::new(0);
    let rereads = AtomicUsize::new(0);
    let (first, second) = tokio::join!(
        coordinator.run(
            key("user", "server"),
            failing_step(&refreshes),
            reread_step(&rereads)
        ),
        coordinator.run(
            key("user", "server"),
            failing_step(&refreshes),
            reread_step(&rereads)
        ),
    );
    let expected = Err(TokenStoreUnavailable {
        detail: "the refresh failed".to_owned(),
    });
    assert_eq!(first, expected);
    assert_eq!(second, expected);
    assert_eq!(refreshes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn coordinator_starts_a_new_refresh_after_the_first_finishes() {
    let coordinator: InProcessRefreshCoordinator<Option<OAuthToken>, TokenStoreUnavailable> =
        InProcessRefreshCoordinator::default();
    let refreshes = AtomicUsize::new(0);
    let rereads = AtomicUsize::new(0);
    coordinator
        .run(
            key("user", "server"),
            refresh_step(&refreshes, "one"),
            reread_step(&rereads),
        )
        .await
        .unwrap();
    coordinator
        .run(
            key("user", "server"),
            refresh_step(&refreshes, "two"),
            reread_step(&rereads),
        )
        .await
        .unwrap();
    assert_eq!(refreshes.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn coordinator_runs_one_refresh_per_key() {
    let coordinator: InProcessRefreshCoordinator<Option<OAuthToken>, TokenStoreUnavailable> =
        InProcessRefreshCoordinator::default();
    let refreshes = AtomicUsize::new(0);
    let rereads = AtomicUsize::new(0);
    let (first, second) = tokio::join!(
        coordinator.run(
            key("user", "first"),
            refresh_step(&refreshes, "one"),
            reread_step(&rereads)
        ),
        coordinator.run(
            key("user", "second"),
            refresh_step(&refreshes, "two"),
            reread_step(&rereads)
        ),
    );
    assert_eq!(first, Ok(Some(token("one", None))));
    assert_eq!(second, Ok(Some(token("two", None))));
    assert_eq!(refreshes.load(Ordering::SeqCst), 2);
}

fn store(
    inner: ScriptedStore,
    refresher: ScriptedRefresher,
    clock: Arc<ManualClock>,
) -> RefreshingTokenStore<
    ScriptedStore,
    ScriptedRefresher,
    InProcessRefreshCoordinator<Option<OAuthToken>, TokenStoreUnavailable>,
> {
    RefreshingTokenStore {
        inner,
        refresher,
        expiry_skew: DEFAULT_EXPIRY_SKEW,
        clock,
        coordinator: InProcessRefreshCoordinator::default(),
    }
}

#[tokio::test]
async fn refreshing_store_returns_an_unexpired_token_untouched() {
    let inner = ScriptedStore::new(vec![Ok(Some(token("access", Some(at(2_000)))))]);
    let refresher = ScriptedRefresher::new(Vec::new());
    let store = store(inner, refresher, clock_at(100));
    assert_eq!(
        store.fetch("user", "server").await,
        Ok(Some(token("access", Some(at(2_000)))))
    );
    assert_eq!(store.refresher.calls.load(Ordering::SeqCst), 0);
    assert_eq!(store.inner.reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn refreshing_store_returns_none_when_the_token_is_absent() {
    let inner = ScriptedStore::new(vec![Ok(None)]);
    let refresher = ScriptedRefresher::new(Vec::new());
    let store = store(inner, refresher, clock_at(100));
    assert_eq!(store.fetch("user", "server").await, Ok(None));
    assert_eq!(store.refresher.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn refreshing_store_refreshes_a_token_at_the_skew_boundary() {
    let inner = ScriptedStore::new(vec![Ok(Some(token("stale", Some(at(1_000)))))]);
    let refresher = ScriptedRefresher::new(vec![Ok(Some(token("fresh", None)))]);
    let store = store(inner, refresher, clock_at(940));
    assert_eq!(
        store.fetch("user", "server").await,
        Ok(Some(token("fresh", None)))
    );
    assert_eq!(store.refresher.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn refreshing_store_keeps_a_token_just_outside_the_skew() {
    let inner = ScriptedStore::new(vec![Ok(Some(token("access", Some(at(1_000)))))]);
    let refresher = ScriptedRefresher::new(Vec::new());
    let store = store(inner, refresher, clock_at(939));
    assert_eq!(
        store.fetch("user", "server").await,
        Ok(Some(token("access", Some(at(1_000)))))
    );
    assert_eq!(store.refresher.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn refreshing_store_returns_none_when_the_refresh_fails() {
    let inner = ScriptedStore::new(vec![Ok(Some(token("stale", Some(at(1_000)))))]);
    let refresher = ScriptedRefresher::new(vec![Ok(None)]);
    let store = store(inner, refresher, clock_at(1_000));
    assert_eq!(store.fetch("user", "server").await, Ok(None));
    assert_eq!(store.refresher.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn refreshing_store_propagates_a_refresh_error() {
    let inner = ScriptedStore::new(vec![Ok(Some(token("stale", Some(at(1_000)))))]);
    let refresher = ScriptedRefresher::new(vec![Err(TokenStoreUnavailable {
        detail: "the store is down".to_owned(),
    })]);
    let store = store(inner, refresher, clock_at(1_000));
    assert_eq!(
        store.fetch("user", "server").await,
        Err(TokenStoreUnavailable {
            detail: "the store is down".to_owned()
        })
    );
}

#[tokio::test]
async fn refreshing_store_rereads_a_persisted_token_when_it_loses_the_refresh() {
    let inner = ScriptedStore::new(vec![
        Ok(Some(token("stale", Some(at(1_000))))),
        Ok(Some(token("fresh", Some(at(2_000))))),
    ]);
    let refresher = ScriptedRefresher::new(Vec::new());
    let store = RefreshingTokenStore {
        inner,
        refresher,
        expiry_skew: DEFAULT_EXPIRY_SKEW,
        clock: clock_at(1_000),
        coordinator: RereadOnly,
    };
    assert_eq!(
        store.fetch("user", "server").await,
        Ok(Some(token("fresh", Some(at(2_000)))))
    );
    assert_eq!(store.refresher.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn refreshing_store_reread_returns_none_for_a_still_expired_token() {
    let inner = ScriptedStore::new(vec![
        Ok(Some(token("stale", Some(at(1_000))))),
        Ok(Some(token("stale", Some(at(1_000))))),
    ]);
    let refresher = ScriptedRefresher::new(Vec::new());
    let store = RefreshingTokenStore {
        inner,
        refresher,
        expiry_skew: DEFAULT_EXPIRY_SKEW,
        clock: clock_at(1_000),
        coordinator: RereadOnly,
    };
    assert_eq!(store.fetch("user", "server").await, Ok(None));
}
