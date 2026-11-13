#![cfg(test)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::task::yield_now;

use super::{InProcessRefreshCoordinator, RefreshingTokenStore};
use crate::mcp::outbound_credentials::{
    DEFAULT_EXPIRY_SKEW, OAuthToken, OAuthTokenStore, RefreshCoordinator, RefreshStep, TokenKey,
    TokenRefresher, TokenStoreUnavailable, WallClock,
};

type Step<'call> = RefreshStep<'call, Option<OAuthToken>, TokenStoreUnavailable>;
type Outcome = Result<Option<OAuthToken>, TokenStoreUnavailable>;

#[derive(Debug)]
struct ManualClock(Mutex<DateTime<Utc>>);

impl WallClock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

struct Scripted {
    results: Mutex<VecDeque<Outcome>>,
    calls: AtomicUsize,
}

impl Scripted {
    fn new(results: Vec<Outcome>) -> Self {
        Self {
            results: Mutex::new(VecDeque::from(results)),
            calls: AtomicUsize::new(0),
        }
    }

    fn next(&self) -> Outcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.results
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("a scripted result per call")
    }
}

#[async_trait]
impl OAuthTokenStore for Scripted {
    async fn fetch(&self, _user_id: &str, _server_id: &str) -> Outcome {
        self.next()
    }
}

#[async_trait]
impl TokenRefresher for Scripted {
    async fn refresh(&self, _user_id: &str, _server_id: &str, _token: &OAuthToken) -> Outcome {
        self.next()
    }
}

struct RereadOnly;

#[async_trait]
impl RefreshCoordinator for RereadOnly {
    type Output = Option<OAuthToken>;
    type Error = TokenStoreUnavailable;

    async fn run(&self, _key: TokenKey, _refresh: Step<'_>, reread: Step<'_>) -> Outcome {
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

fn scripted_step(calls: &AtomicUsize, result: Outcome) -> Step<'_> {
    Box::new(move || {
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            yield_now().await;
            result.clone()
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
            scripted_step(&refreshes, Ok(Some(token("one", None)))),
            scripted_step(&rereads, Ok(None))
        ),
        coordinator.run(
            key("user", "server"),
            scripted_step(&refreshes, Ok(Some(token("one", None)))),
            scripted_step(&rereads, Ok(None))
        ),
        coordinator.run(
            key("user", "server"),
            scripted_step(&refreshes, Ok(Some(token("one", None)))),
            scripted_step(&rereads, Ok(None))
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
    let failed = Err(TokenStoreUnavailable {
        detail: "the refresh failed".to_owned(),
    });
    let (first, second) = tokio::join!(
        coordinator.run(
            key("user", "server"),
            scripted_step(&refreshes, failed.clone()),
            scripted_step(&rereads, Ok(None))
        ),
        coordinator.run(
            key("user", "server"),
            scripted_step(&refreshes, failed),
            scripted_step(&rereads, Ok(None))
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
            scripted_step(&refreshes, Ok(Some(token("one", None)))),
            scripted_step(&rereads, Ok(None)),
        )
        .await
        .unwrap();
    coordinator
        .run(
            key("user", "server"),
            scripted_step(&refreshes, Ok(Some(token("two", None)))),
            scripted_step(&rereads, Ok(None)),
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
            scripted_step(&refreshes, Ok(Some(token("one", None)))),
            scripted_step(&rereads, Ok(None))
        ),
        coordinator.run(
            key("user", "second"),
            scripted_step(&refreshes, Ok(Some(token("two", None)))),
            scripted_step(&rereads, Ok(None))
        ),
    );
    assert_eq!(first, Ok(Some(token("one", None))));
    assert_eq!(second, Ok(Some(token("two", None))));
    assert_eq!(refreshes.load(Ordering::SeqCst), 2);
}

fn store(
    inner: Scripted,
    refresher: Scripted,
    clock: Arc<ManualClock>,
) -> RefreshingTokenStore<
    Scripted,
    Scripted,
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
    let inner = Scripted::new(vec![Ok(Some(token("access", Some(at(2_000)))))]);
    let refresher = Scripted::new(Vec::new());
    let store = store(inner, refresher, clock_at(100));
    assert_eq!(
        store.fetch("user", "server").await,
        Ok(Some(token("access", Some(at(2_000)))))
    );
    assert_eq!(store.refresher.calls.load(Ordering::SeqCst), 0);
    assert_eq!(store.inner.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn refreshing_store_returns_none_when_the_token_is_absent() {
    let inner = Scripted::new(vec![Ok(None)]);
    let refresher = Scripted::new(Vec::new());
    let store = store(inner, refresher, clock_at(100));
    assert_eq!(store.fetch("user", "server").await, Ok(None));
    assert_eq!(store.refresher.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn refreshing_store_refreshes_a_token_at_the_skew_boundary() {
    let inner = Scripted::new(vec![Ok(Some(token("stale", Some(at(1_000)))))]);
    let refresher = Scripted::new(vec![Ok(Some(token("fresh", None)))]);
    let store = store(inner, refresher, clock_at(940));
    assert_eq!(
        store.fetch("user", "server").await,
        Ok(Some(token("fresh", None)))
    );
    assert_eq!(store.refresher.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn refreshing_store_keeps_a_token_just_outside_the_skew() {
    let inner = Scripted::new(vec![Ok(Some(token("access", Some(at(1_000)))))]);
    let refresher = Scripted::new(Vec::new());
    let store = store(inner, refresher, clock_at(939));
    assert_eq!(
        store.fetch("user", "server").await,
        Ok(Some(token("access", Some(at(1_000)))))
    );
    assert_eq!(store.refresher.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn refreshing_store_returns_none_when_the_refresh_fails() {
    let inner = Scripted::new(vec![Ok(Some(token("stale", Some(at(1_000)))))]);
    let refresher = Scripted::new(vec![Ok(None)]);
    let store = store(inner, refresher, clock_at(1_000));
    assert_eq!(store.fetch("user", "server").await, Ok(None));
    assert_eq!(store.refresher.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn refreshing_store_propagates_a_refresh_error() {
    let inner = Scripted::new(vec![Ok(Some(token("stale", Some(at(1_000)))))]);
    let refresher = Scripted::new(vec![Err(TokenStoreUnavailable {
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
    let inner = Scripted::new(vec![
        Ok(Some(token("stale", Some(at(1_000))))),
        Ok(Some(token("fresh", Some(at(2_000))))),
    ]);
    let refresher = Scripted::new(Vec::new());
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
    let inner = Scripted::new(vec![
        Ok(Some(token("stale", Some(at(1_000))))),
        Ok(Some(token("stale", Some(at(1_000))))),
    ]);
    let refresher = Scripted::new(Vec::new());
    let store = RefreshingTokenStore {
        inner,
        refresher,
        expiry_skew: DEFAULT_EXPIRY_SKEW,
        clock: clock_at(1_000),
        coordinator: RereadOnly,
    };
    assert_eq!(store.fetch("user", "server").await, Ok(None));
}
