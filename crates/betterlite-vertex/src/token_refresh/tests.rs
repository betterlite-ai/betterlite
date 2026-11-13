#![cfg(test)]

mod support;

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, SystemTime};

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::sync::{Notify, watch};
use tokio::task::{JoinSet, yield_now};
use tokio::time::timeout;
use tokio_util::task::TaskTracker;

use crate::credential_sources::sdk::CredentialClock;
use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::token_cache::CredentialCache;
use crate::token_cache::error::{CredentialCacheError, needs_reauthentication};
use crate::token_cache::executor::{CredentialExecutor, LoadedCredential};
use crate::token_cache::key::CredentialKey;
use crate::token_cache::probe::AccessToken;
use crate::token_cache::record::{CacheRecord, CredentialSnapshot};
use crate::token_cache::state::TokenState;

use self::support::{
    ControlledClock, ControlledExecutor, RefreshReply, expiring, loaded, network_error,
    reauthentication_error, snapshot,
};
use super::background::{RefreshJob, await_background_refresh, release_job};
use super::{ensure_access_token, get_access_token_async};

const CREDENTIAL: &str = "credential-secret-value";
const PROJECT: &str = "project-p";
const WAIT: Duration = Duration::from_secs(10);

struct Fixture {
    cache: Arc<CredentialCache>,
    clock: Arc<ControlledClock>,
    executor: Arc<ControlledExecutor>,
    tasks: TaskTracker,
}

fn credentials() -> ConfiguredCredentials {
    ConfiguredCredentials::Text(SecretString::from(CREDENTIAL))
}

fn requested_key(project: Option<&str>) -> CredentialKey {
    CredentialKey::new(Some(&credentials()), project)
}

fn now_plus(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .unwrap()
}

fn new_fixture(
    loads: Vec<Result<LoadedCredential, CredentialCacheError>>,
    refreshes: Vec<RefreshReply>,
) -> Fixture {
    let clock = Arc::new(ControlledClock::at(SystemTime::UNIX_EPOCH));
    let executor = Arc::new(ControlledExecutor::scripted(loads, refreshes));
    let clock_handle = Arc::clone(&clock);
    let clock_source: Arc<dyn CredentialClock> = clock_handle;
    let executor_handle = Arc::clone(&executor);
    let credential_executor: Arc<dyn CredentialExecutor> = executor_handle;
    let tasks = TaskTracker::new();
    let cache = CredentialCache::new(clock_source, credential_executor, tasks.clone());
    Fixture {
        cache: Arc::new(cache),
        clock,
        executor,
        tasks,
    }
}

async fn drain(fixture: &Fixture) {
    fixture.tasks.close();
    fixture.tasks.wait().await;
}

fn staged(
    started: &Arc<Notify>,
    gate: &Arc<Notify>,
    reply: Result<CredentialSnapshot, CredentialCacheError>,
) -> RefreshReply {
    RefreshReply::Gated {
        started: Arc::clone(started),
        gate: Arc::clone(gate),
        reply,
    }
}

fn fresh(token: &str) -> CredentialSnapshot {
    snapshot(Some(token), TokenState::Fresh, Some(PROJECT))
}

fn store_handle(
    cache: &CredentialCache,
    key: &CredentialKey,
    credential: &Arc<CredentialSnapshot>,
    project: Option<&str>,
) {
    cache.store_record(
        key.clone(),
        CacheRecord::Paired {
            credential: Arc::clone(credential),
            project: project.map(str::to_owned),
        },
    );
}

fn store(
    cache: &CredentialCache,
    key: &CredentialKey,
    snapshot: CredentialSnapshot,
    project: Option<&str>,
) -> Arc<CredentialSnapshot> {
    let credential = Arc::new(snapshot);
    store_handle(cache, key, &credential, project);
    credential
}

fn store_project(
    cache: &CredentialCache,
    key: &CredentialKey,
    token: &str,
    state: TokenState,
) -> Arc<CredentialSnapshot> {
    store(
        cache,
        key,
        snapshot(Some(token), state, Some(PROJECT)),
        Some(PROJECT),
    )
}

async fn lookup(
    fixture: &Fixture,
    project: Option<&str>,
) -> Result<AccessToken, CredentialCacheError> {
    get_access_token_async(&fixture.cache, Some(&credentials()), project).await
}

fn refreshing_identities(cache: &CredentialCache) -> Vec<CredentialKey> {
    cache.refreshes.lock().unwrap().keys().cloned().collect()
}

fn cached_project(record: &CacheRecord) -> Option<String> {
    match record {
        CacheRecord::Paired { project, .. } => project.clone(),
        CacheRecord::Legacy { .. } => None,
    }
}

fn token_of(record: &CacheRecord) -> String {
    record
        .credential()
        .token
        .as_ref()
        .unwrap()
        .expose_secret()
        .to_owned()
}

fn poll_once<F>(mut future: Pin<&mut F>) -> Poll<F::Output>
where
    F: Future,
{
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    future.as_mut().poll(&mut context)
}

#[derive(Clone, Copy)]
enum Scenario {
    StaleReturnsNow,
    InvalidJoinsRefresh,
    FailedBackgroundRetry,
    ReplacedHandleSurvives,
    ReauthAliasReplaced,
    GeminiSkipsAuth,
}

#[rstest]
#[case::stale_returns_now(Scenario::StaleReturnsNow)]
#[case::invalid_joins_refresh(Scenario::InvalidJoinsRefresh)]
#[case::failed_background_retry(Scenario::FailedBackgroundRetry)]
#[case::replaced_handle_survives(Scenario::ReplacedHandleSurvives)]
#[case::reauth_alias_replaced(Scenario::ReauthAliasReplaced)]
#[case::gemini_skips_auth(Scenario::GeminiSkipsAuth)]
#[tokio::test]
async fn refresh_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::StaleReturnsNow => stale_returns_now().await,
        Scenario::InvalidJoinsRefresh => invalid_joins_refresh().await,
        Scenario::FailedBackgroundRetry => failed_background_retry().await,
        Scenario::ReplacedHandleSurvives => replaced_handle_survives().await,
        Scenario::ReauthAliasReplaced => reauth_alias_replaced().await,
        Scenario::GeminiSkipsAuth => gemini_skips_auth().await,
    }
}

async fn stale_returns_now() {
    let started = Arc::new(Notify::new());
    let gate = Arc::new(Notify::new());
    let fixture = new_fixture(
        Vec::new(),
        vec![staged(&started, &gate, Ok(fresh("token-fresh")))],
    );
    let key = requested_key(Some(PROJECT));
    let stale = store_project(&fixture.cache, &key, "token-stale", TokenState::Stale);
    let mut first_lookup = Box::pin(lookup(&fixture, Some(PROJECT)));
    let Poll::Ready(Ok(served)) = poll_once(first_lookup.as_mut()) else {
        panic!("a stale cached token is served without waiting for the refresh");
    };
    assert_eq!(served.token.expose_secret(), "token-stale");
    assert_eq!(served.project, PROJECT);
    let again = lookup(&fixture, Some(PROJECT)).await.unwrap();
    assert_eq!(again.token.expose_secret(), "token-stale");
    timeout(WAIT, started.notified()).await.unwrap();
    gate.notify_one();
    drain(&fixture).await;
    let refreshed = fixture.cache.cached_record(&key).unwrap();
    assert_eq!(token_of(&refreshed), "token-fresh");
    assert!(!Arc::ptr_eq(refreshed.credential(), &stale));
    assert!(refreshing_identities(&fixture.cache).is_empty());
    assert_eq!(
        fixture.executor.refresh_targets()[0]
            .as_ref()
            .unwrap()
            .expose_secret(),
        "token-stale"
    );
    assert_eq!(fixture.executor.refresh_count(), 1);
}

async fn invalid_joins_refresh() {
    let started = Arc::new(Notify::new());
    let gate = Arc::new(Notify::new());
    let fixture = new_fixture(
        Vec::new(),
        vec![staged(&started, &gate, Ok(fresh("token-fresh")))],
    );
    let key = requested_key(Some(PROJECT));
    store(
        &fixture.cache,
        &key,
        expiring("token-stale", now_plus(100)),
        Some(PROJECT),
    );
    let first = lookup(&fixture, Some(PROJECT)).await.unwrap();
    assert_eq!(first.token.expose_secret(), "token-stale");
    timeout(WAIT, started.notified()).await.unwrap();
    fixture.clock.set(now_plus(1000));
    let mut joined = Box::pin(lookup(&fixture, Some(PROJECT)));
    assert!(poll_once(joined.as_mut()).is_pending());
    gate.notify_one();
    let second = joined.await.unwrap();
    assert_eq!(second.token.expose_secret(), "token-fresh");
    assert_eq!(second.project, PROJECT);
    assert_eq!(fixture.executor.refresh_count(), 1);
    assert!(fixture.cache.leased_identities().is_empty());
    drain(&fixture).await;
}

async fn failed_background_retry() {
    let started = Arc::new(Notify::new());
    let gate = Arc::new(Notify::new());
    let fixture = new_fixture(
        Vec::new(),
        vec![
            staged(&started, &gate, Err(network_error())),
            RefreshReply::Immediate(Ok(fresh("token-fresh"))),
        ],
    );
    let key = requested_key(Some(PROJECT));
    store_project(&fixture.cache, &key, "token-stale", TokenState::Stale);
    let first = lookup(&fixture, Some(PROJECT)).await.unwrap();
    assert_eq!(first.token.expose_secret(), "token-stale");
    timeout(WAIT, started.notified()).await.unwrap();
    gate.notify_one();
    await_background_refresh(&fixture.cache, &key).await;
    assert_eq!(fixture.executor.refresh_count(), 1);
    assert_eq!(
        token_of(&fixture.cache.cached_record(&key).unwrap()),
        "token-stale"
    );
    let second = lookup(&fixture, Some(PROJECT)).await.unwrap();
    assert_eq!(second.token.expose_secret(), "token-stale");
    await_background_refresh(&fixture.cache, &key).await;
    assert_eq!(fixture.executor.refresh_count(), 2);
    assert_eq!(
        token_of(&fixture.cache.cached_record(&key).unwrap()),
        "token-fresh"
    );
    drain(&fixture).await;
}

async fn replaced_handle_survives() {
    let started = Arc::new(Notify::new());
    let gate = Arc::new(Notify::new());
    let fixture = new_fixture(
        Vec::new(),
        vec![staged(&started, &gate, Ok(fresh("token-refreshed")))],
    );
    let key = requested_key(Some(PROJECT));
    store_project(&fixture.cache, &key, "token-stale", TokenState::Stale);
    let served = lookup(&fixture, Some(PROJECT)).await.unwrap();
    assert_eq!(served.token.expose_secret(), "token-stale");
    timeout(WAIT, started.notified()).await.unwrap();
    let replacement = Arc::new(snapshot(
        Some("token-kept"),
        TokenState::Fresh,
        Some(PROJECT),
    ));
    store_handle(&fixture.cache, &key, &replacement, Some(PROJECT));
    gate.notify_one();
    await_background_refresh(&fixture.cache, &key).await;
    let cached = fixture.cache.cached_record(&key).unwrap();
    assert_eq!(token_of(&cached), "token-kept");
    assert!(Arc::ptr_eq(cached.credential(), &replacement));
    assert_eq!(fixture.executor.refresh_count(), 1);
    drain(&fixture).await;
}

async fn reauth_alias_replaced() {
    let fixture = new_fixture(
        vec![Ok(loaded(
            "token-reloaded",
            TokenState::Fresh,
            Some(PROJECT),
        ))],
        vec![RefreshReply::Immediate(Err(reauthentication_error()))],
    );
    let requested = requested_key(None);
    let alias = requested_key(Some(PROJECT));
    store(
        &fixture.cache,
        &requested,
        snapshot(Some("token-old"), TokenState::Invalid, None),
        None,
    );
    let old_alias = store(
        &fixture.cache,
        &alias,
        snapshot(Some("token-alias-old"), TokenState::Fresh, Some(PROJECT)),
        Some(PROJECT),
    );
    let granted = lookup(&fixture, None).await.unwrap();
    assert_eq!(granted.token.expose_secret(), "token-reloaded");
    assert_eq!(granted.project, PROJECT);
    let replaced = fixture.cache.cached_record(&alias).unwrap();
    assert_eq!(token_of(&replaced), "token-reloaded");
    assert!(!Arc::ptr_eq(replaced.credential(), &old_alias));
    assert_eq!(fixture.executor.load_count(), 1);
    assert_eq!(fixture.executor.refresh_count(), 1);
    drain(&fixture).await;
}

async fn gemini_skips_auth() {
    let fixture = new_fixture(Vec::new(), Vec::new());
    let configured = credentials();
    let synchronous = ensure_access_token(
        VertexCapabilityProvider::Gemini,
        fixture
            .cache
            .get_access_token(Some(&configured), Some(PROJECT)),
    )
    .await
    .unwrap();
    let asynchronous = ensure_access_token(
        VertexCapabilityProvider::Gemini,
        get_access_token_async(&fixture.cache, Some(&configured), Some(PROJECT)),
    )
    .await
    .unwrap();
    assert_eq!(synchronous.token.expose_secret(), "");
    assert_eq!(synchronous.project, "");
    assert_eq!(asynchronous.token.expose_secret(), "");
    assert_eq!(asynchronous.project, "");
    assert_eq!(fixture.executor.load_count(), 0);
    assert_eq!(fixture.executor.refresh_count(), 0);
    assert!(fixture.cache.cached_record(&requested_key(None)).is_none());
    let error = ensure_access_token(
        VertexCapabilityProvider::VertexAi,
        fixture
            .cache
            .get_access_token(Some(&configured), Some(PROJECT)),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, CredentialCacheError::Load(_)));
    assert_eq!(fixture.executor.load_count(), 1);
}

#[tokio::test]
async fn shutdown_drains_refresh() {
    let started = Arc::new(Notify::new());
    let gate = Arc::new(Notify::new());
    let fixture = new_fixture(
        Vec::new(),
        vec![staged(&started, &gate, Ok(fresh("token-fresh")))],
    );
    let key = requested_key(Some(PROJECT));
    store_project(&fixture.cache, &key, "token-stale", TokenState::Stale);
    let served = lookup(&fixture, Some(PROJECT)).await.unwrap();
    assert_eq!(served.token.expose_secret(), "token-stale");
    timeout(WAIT, started.notified()).await.unwrap();
    let mut drains = JoinSet::new();
    let draining = fixture.tasks.clone();
    drains.spawn(async move {
        draining.close();
        draining.wait().await;
    });
    yield_now().await;
    assert!(!drains.is_empty());
    gate.notify_one();
    timeout(WAIT, drains.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(fixture.executor.refresh_count(), 1);
    assert_eq!(
        token_of(&fixture.cache.cached_record(&key).unwrap()),
        "token-fresh"
    );
    assert!(refreshing_identities(&fixture.cache).is_empty());
    store_project(&fixture.cache, &key, "token-late", TokenState::Stale);
    let late = lookup(&fixture, Some(PROJECT)).await.unwrap();
    assert_eq!(late.token.expose_secret(), "token-late");
    yield_now().await;
    assert_eq!(fixture.executor.refresh_count(), 1);
}

#[tokio::test]
async fn cancelled_waiter_releases_key() {
    let started = Arc::new(Notify::new());
    let gate = Arc::new(Notify::new());
    let fixture = new_fixture(
        Vec::new(),
        vec![staged(&started, &gate, Ok(fresh("token-fresh")))],
    );
    let key = requested_key(Some(PROJECT));
    store_project(&fixture.cache, &key, "token-old", TokenState::Invalid);
    let configured = credentials();
    let mut first = Box::pin(get_access_token_async(
        &fixture.cache,
        Some(&configured),
        Some(PROJECT),
    ));
    assert!(poll_once(first.as_mut()).is_pending());
    timeout(WAIT, started.notified()).await.unwrap();
    let mut second = Box::pin(get_access_token_async(
        &fixture.cache,
        Some(&configured),
        Some(PROJECT),
    ));
    assert!(poll_once(second.as_mut()).is_pending());
    assert_eq!(fixture.cache.leased_identities(), vec![key.clone()]);
    drop(second);
    assert_eq!(fixture.cache.leased_identities(), vec![key.clone()]);
    assert_eq!(fixture.executor.refresh_count(), 1);
    gate.notify_one();
    let granted = first.await.unwrap();
    assert_eq!(granted.token.expose_secret(), "token-fresh");
    assert!(fixture.cache.leased_identities().is_empty());
    drain(&fixture).await;
}

#[test]
fn older_task_cleanup_keeps_its_replacement() {
    let key = requested_key(None);
    let (_sender, done) = watch::channel(false);
    let mut jobs = BTreeMap::from([(key.clone(), RefreshJob { id: 2, done })]);
    release_job(&mut jobs, &key, 1);
    assert!(jobs.contains_key(&key));
    release_job(&mut jobs, &key, 2);
    assert!(jobs.is_empty());
}

#[tokio::test]
async fn fresh_token_skips_the_lease() {
    let started = Arc::new(Notify::new());
    let gate = Arc::new(Notify::new());
    let fixture = new_fixture(
        Vec::new(),
        vec![staged(&started, &gate, Ok(fresh("token-refreshed")))],
    );
    let key = requested_key(Some(PROJECT));
    store_project(&fixture.cache, &key, "token-old", TokenState::Invalid);
    let configured = credentials();
    let mut blocking = Box::pin(get_access_token_async(
        &fixture.cache,
        Some(&configured),
        Some(PROJECT),
    ));
    assert!(poll_once(blocking.as_mut()).is_pending());
    timeout(WAIT, started.notified()).await.unwrap();
    store_project(&fixture.cache, &key, "token-fresh", TokenState::Fresh);
    let mut fast_lookup = Box::pin(get_access_token_async(
        &fixture.cache,
        Some(&configured),
        Some(PROJECT),
    ));
    let Poll::Ready(Ok(served)) = poll_once(fast_lookup.as_mut()) else {
        panic!("a fresh cached token must not wait for the refresh lease");
    };
    assert_eq!(served.token.expose_secret(), "token-fresh");
    drop(fast_lookup);
    assert_eq!(fixture.cache.leased_identities(), vec![key.clone()]);
    gate.notify_one();
    blocking.await.unwrap();
    drain(&fixture).await;
}

#[tokio::test]
async fn resolved_alias_written_on_slow_path() {
    let fixture = new_fixture(
        Vec::new(),
        vec![RefreshReply::Immediate(Ok(fresh("token-fresh")))],
    );
    let unresolved = requested_key(None);
    store(
        &fixture.cache,
        &unresolved,
        snapshot(Some("token-old"), TokenState::Invalid, Some(PROJECT)),
        Some(PROJECT),
    );
    let granted = lookup(&fixture, None).await.unwrap();
    assert_eq!(granted.token.expose_secret(), "token-fresh");
    assert_eq!(granted.project, PROJECT);
    let alias = fixture
        .cache
        .cached_record(&requested_key(Some(PROJECT)))
        .unwrap();
    assert_eq!(cached_project(&alias), Some(PROJECT.to_owned()));
    assert_eq!(fixture.executor.refresh_count(), 1);
    drain(&fixture).await;
}

#[tokio::test]
async fn blocking_refresh_error_is_not_retried() {
    let fixture = new_fixture(
        Vec::new(),
        vec![RefreshReply::Immediate(Err(network_error()))],
    );
    let key = requested_key(Some(PROJECT));
    store_project(&fixture.cache, &key, "token-old", TokenState::Invalid);
    let error = lookup(&fixture, Some(PROJECT)).await.unwrap_err();
    assert!(!needs_reauthentication(&error));
    assert!(matches!(error, CredentialCacheError::Refresh(_)));
    assert_eq!(fixture.executor.load_count(), 0);
    assert_eq!(fixture.executor.refresh_count(), 1);
    assert!(fixture.cache.cached_record(&key).is_some());
    drain(&fixture).await;
}

#[tokio::test]
async fn vertex_wrappers_read_credentials() {
    let fixture = new_fixture(Vec::new(), Vec::new());
    let configured = credentials();
    let synchronous = ensure_access_token(
        VertexCapabilityProvider::VertexAi,
        fixture
            .cache
            .get_access_token(Some(&configured), Some(PROJECT)),
    )
    .await;
    assert!(matches!(synchronous, Err(CredentialCacheError::Load(_))));
    let asynchronous = ensure_access_token(
        VertexCapabilityProvider::VertexAiBeta,
        get_access_token_async(&fixture.cache, Some(&configured), Some(PROJECT)),
    )
    .await;
    assert!(matches!(asynchronous, Err(CredentialCacheError::Load(_))));
    assert_eq!(fixture.executor.load_count(), 2);
    drain(&fixture).await;
}

#[tokio::test]
async fn reauth_keeps_the_requested_project() {
    let fixture = new_fixture(
        vec![Ok(loaded(
            "token-reloaded",
            TokenState::Fresh,
            Some("loaded-project"),
        ))],
        vec![RefreshReply::Immediate(Err(reauthentication_error()))],
    );
    let key = requested_key(Some(PROJECT));
    store_project(&fixture.cache, &key, "token-old", TokenState::Invalid);
    let granted = lookup(&fixture, Some(PROJECT)).await.unwrap();
    assert_eq!(granted.token.expose_secret(), "token-reloaded");
    assert_eq!(granted.project, PROJECT);
    assert!(
        fixture
            .cache
            .cached_record(&requested_key(Some("loaded-project")))
            .is_none()
    );
    assert_eq!(fixture.executor.load_count(), 1);
    drain(&fixture).await;
}

#[tokio::test]
async fn injected_tracker_drains_refresh() {
    let started = Arc::new(Notify::new());
    let gate = Arc::new(Notify::new());
    let fixture = new_fixture(
        Vec::new(),
        vec![staged(&started, &gate, Ok(fresh("token-fresh")))],
    );
    let key = requested_key(Some(PROJECT));
    store_project(&fixture.cache, &key, "token-stale", TokenState::Stale);
    let served = lookup(&fixture, Some(PROJECT)).await.unwrap();
    assert_eq!(served.token.expose_secret(), "token-stale");
    timeout(WAIT, started.notified()).await.unwrap();
    gate.notify_one();
    fixture.tasks.close();
    fixture.tasks.wait().await;
    assert_eq!(
        token_of(&fixture.cache.cached_record(&key).unwrap()),
        "token-fresh"
    );
    assert_eq!(fixture.executor.refresh_count(), 1);
    drain(&fixture).await;
}

#[tokio::test]
async fn stale_record_without_a_token_blocks_on_refresh() {
    let fixture = new_fixture(
        Vec::new(),
        vec![RefreshReply::Immediate(Ok(fresh("token-fresh")))],
    );
    let key = requested_key(Some(PROJECT));
    store(
        &fixture.cache,
        &key,
        snapshot(None, TokenState::Stale, Some(PROJECT)),
        Some(PROJECT),
    );
    let granted = lookup(&fixture, Some(PROJECT)).await.unwrap();
    assert_eq!(granted.token.expose_secret(), "token-fresh");
    assert_eq!(granted.project, PROJECT);
    assert_eq!(fixture.executor.refresh_count(), 1);
    drain(&fixture).await;
}

#[tokio::test]
async fn async_lookup_reports_missing_token_and_project() {
    let fixture = new_fixture(Vec::new(), Vec::new());
    let key = requested_key(Some(PROJECT));
    store(
        &fixture.cache,
        &key,
        snapshot(None, TokenState::Fresh, Some(PROJECT)),
        Some(PROJECT),
    );
    let missing_token = lookup(&fixture, Some(PROJECT)).await.unwrap_err();
    assert!(matches!(missing_token, CredentialCacheError::MissingToken));
    let unresolved = requested_key(None);
    store(
        &fixture.cache,
        &unresolved,
        snapshot(Some("token-a"), TokenState::Fresh, None),
        None,
    );
    let missing_project = lookup(&fixture, None).await.unwrap_err();
    assert!(matches!(
        missing_project,
        CredentialCacheError::MissingProject
    ));
    drain(&fixture).await;
}
