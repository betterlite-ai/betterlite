#![cfg(test)]

mod probes;
mod unpacking;

use std::collections::VecDeque;
use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, SystemTime};

use betterlite_config::sections::vertex_credential_sources::credential_source::{
    ConfiguredCredentials, decode_vertex_credentials,
};
use google_cloud_auth::errors::CredentialsError;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::sync::Notify;
use tokio_util::task::TaskTracker;

use crate::credential_sources::error::CredentialSourceError;
use crate::credential_sources::sdk::CredentialClock;

use super::CredentialCache;
use super::error::{CredentialCacheError, needs_reauthentication};
use super::executor::{CredentialExecutor, LoadedCredential};
use super::key::CredentialKey;
use super::probe::AccessToken;
use super::record::{CacheRecord, CredentialSnapshot};
use super::state::TokenState;

const SECRET_TOKEN: &str = "token-secret-value";
const SECRET_CREDENTIAL: &str = "credential-secret-value";

struct FixedClock(SystemTime);

impl CredentialClock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

enum RefreshReply {
    Immediate(Result<CredentialSnapshot, CredentialCacheError>),
    Gated {
        started: Arc<Notify>,
        gate: Arc<Notify>,
        reply: Result<CredentialSnapshot, CredentialCacheError>,
    },
}

struct FakeExecutor {
    loads: Mutex<VecDeque<Result<LoadedCredential, CredentialCacheError>>>,
    refreshes: Mutex<VecDeque<RefreshReply>>,
    load_calls: AtomicUsize,
    refresh_calls: AtomicUsize,
    repeat_reauth: bool,
}

impl FakeExecutor {
    fn scripted(
        loads: Vec<Result<LoadedCredential, CredentialCacheError>>,
        refreshes: Vec<RefreshReply>,
    ) -> Self {
        Self {
            loads: Mutex::new(VecDeque::from(loads)),
            refreshes: Mutex::new(VecDeque::from(refreshes)),
            load_calls: AtomicUsize::new(0),
            refresh_calls: AtomicUsize::new(0),
            repeat_reauth: false,
        }
    }

    // every later load and refresh repeats the reauthentication failure, so a retry that loops never ends
    fn repeating_reauth() -> Self {
        Self {
            loads: Mutex::new(VecDeque::new()),
            refreshes: Mutex::new(VecDeque::new()),
            load_calls: AtomicUsize::new(0),
            refresh_calls: AtomicUsize::new(0),
            repeat_reauth: true,
        }
    }

    fn load_count(&self) -> usize {
        self.load_calls.load(AtomicOrdering::SeqCst)
    }

    fn refresh_count(&self) -> usize {
        self.refresh_calls.load(AtomicOrdering::SeqCst)
    }

    fn default_load(&self) -> Result<LoadedCredential, CredentialCacheError> {
        if self.repeat_reauth {
            return Ok(loaded("token-old", TokenState::Invalid, Some("project-p")));
        }
        Err(load_error())
    }

    async fn answer(
        &self,
        next: Option<RefreshReply>,
    ) -> Result<CredentialSnapshot, CredentialCacheError> {
        match next {
            Some(RefreshReply::Gated {
                started,
                gate,
                reply,
            }) => {
                started.notify_one();
                gate.notified().await;
                reply
            }
            Some(RefreshReply::Immediate(reply)) => reply,
            None if self.repeat_reauth => Err(reauthentication_error()),
            None => Err(network_error()),
        }
    }
}

impl CredentialExecutor for FakeExecutor {
    fn load<'borrow>(
        &'borrow self,
        _credentials: Option<&'borrow ConfiguredCredentials>,
        _project: Option<&'borrow str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<LoadedCredential, CredentialCacheError>> + Send + 'borrow>,
    > {
        self.load_calls.fetch_add(1, AtomicOrdering::SeqCst);
        let next = self.loads.lock().unwrap().pop_front();
        let reply = next.unwrap_or_else(|| self.default_load());
        Box::pin(async move { reply })
    }

    fn refresh<'borrow>(
        &'borrow self,
        _credential: &'borrow CredentialSnapshot,
    ) -> Pin<
        Box<dyn Future<Output = Result<CredentialSnapshot, CredentialCacheError>> + Send + 'borrow>,
    > {
        self.refresh_calls.fetch_add(1, AtomicOrdering::SeqCst);
        let next = self.refreshes.lock().unwrap().pop_front();
        Box::pin(self.answer(next))
    }
}

fn cache_with(executor: Arc<FakeExecutor>) -> CredentialCache {
    CredentialCache::new(
        Arc::new(FixedClock(SystemTime::UNIX_EPOCH)),
        executor,
        TaskTracker::new(),
    )
}

fn credential_text() -> ConfiguredCredentials {
    ConfiguredCredentials::Text(SecretString::from(SECRET_CREDENTIAL))
}

fn service_account(project: &str) -> ConfiguredCredentials {
    let document = format!(
        r#"{{"type":"service_account","project_id":"{project}","private_key":"-----BEGIN PRIVATE KEY-----\nkey\n-----END PRIVATE KEY-----\n","client_email":"service@example.com"}}"#
    );
    let source = decode_vertex_credentials(document.as_bytes()).unwrap();
    ConfiguredCredentials::Source(source)
}

fn snapshot(token: Option<&str>, state: TokenState, project: Option<&str>) -> CredentialSnapshot {
    CredentialSnapshot {
        token: token.map(SecretString::from),
        state: Some(state),
        expires_at: None,
        expired: None,
        valid: None,
        quota_project_id: None,
        project_id: project.map(str::to_owned),
    }
}

fn legacy_snapshot(quota: Option<&str>, project: Option<&str>) -> CredentialSnapshot {
    let mut snapshot = snapshot(Some("token-legacy"), TokenState::Fresh, project);
    snapshot.quota_project_id = quota.map(str::to_owned);
    snapshot
}

fn loaded_from(snapshot: CredentialSnapshot, project: Option<&str>) -> LoadedCredential {
    LoadedCredential {
        credential: Some(Arc::new(snapshot)),
        project: project.map(str::to_owned),
    }
}

fn loaded(token: &str, state: TokenState, project: Option<&str>) -> LoadedCredential {
    loaded_from(snapshot(Some(token), state, project), project)
}

fn reauthentication_error() -> CredentialCacheError {
    CredentialCacheError::Refresh(CredentialSourceError::Refresh(CredentialsError::from_msg(
        false,
        "Reauthentication is needed. Please run `gcloud auth application-default login`",
    )))
}

fn network_error() -> CredentialCacheError {
    CredentialCacheError::Refresh(CredentialSourceError::Refresh(CredentialsError::from_msg(
        false,
        "the token endpoint timed out",
    )))
}

fn load_error() -> CredentialCacheError {
    CredentialCacheError::Load(CredentialSourceError::MissingProject)
}

fn carries(error: &CredentialCacheError, needle: &str) -> bool {
    let mut source: Option<&dyn Error> = Some(error);
    while let Some(current) = source {
        if current.to_string().contains(needle) {
            return true;
        }
        source = current.source();
    }
    false
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
    ProjectAliasRetained,
    ReloadOnce,
    OriginalErrorSurvives,
    CachedTokenSkipsRefresh,
    AdcEntryIsReused,
    DistinctCredentialsLoadSeparately,
    SerializedSourcesAreDistinct,
    LegacyRecordProject,
    RefreshedRecordKeepsProject,
    MissingCredentials,
    MissingToken,
    MissingProject,
    UnrelatedRefreshErrorIsNotRetried,
    StaleTokenIsRefreshed,
    ExpiringTokenIsRefreshed,
    ExpiredFalseWithValidFalseSkipsRefresh,
    FallbackFlagsMarkFresh,
    ResolvedAliasServesExplicitProject,
}

#[rstest]
#[case::project_alias_retained(Scenario::ProjectAliasRetained)]
#[case::reload_once(Scenario::ReloadOnce)]
#[case::original_error_survives(Scenario::OriginalErrorSurvives)]
#[case::cached_token_skips_refresh(Scenario::CachedTokenSkipsRefresh)]
#[case::adc_entry_is_reused(Scenario::AdcEntryIsReused)]
#[case::distinct_credentials(Scenario::DistinctCredentialsLoadSeparately)]
#[case::serialized_sources(Scenario::SerializedSourcesAreDistinct)]
#[case::legacy_record_project(Scenario::LegacyRecordProject)]
#[case::refreshed_record_keeps_project(Scenario::RefreshedRecordKeepsProject)]
#[case::missing_credentials(Scenario::MissingCredentials)]
#[case::missing_token(Scenario::MissingToken)]
#[case::missing_project(Scenario::MissingProject)]
#[case::unrelated_refresh_error(Scenario::UnrelatedRefreshErrorIsNotRetried)]
#[case::stale_token_refreshed(Scenario::StaleTokenIsRefreshed)]
#[case::expiring_token_refreshed(Scenario::ExpiringTokenIsRefreshed)]
#[case::expired_false_with_valid_false(Scenario::ExpiredFalseWithValidFalseSkipsRefresh)]
#[case::fallback_flags(Scenario::FallbackFlagsMarkFresh)]
#[case::resolved_alias(Scenario::ResolvedAliasServesExplicitProject)]
#[tokio::test]
async fn cache_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::ProjectAliasRetained => project_alias_retained().await,
        Scenario::ReloadOnce => reload_once().await,
        Scenario::OriginalErrorSurvives => original_error_survives().await,
        Scenario::CachedTokenSkipsRefresh => cached_token_skips_refresh().await,
        Scenario::AdcEntryIsReused => adc_entry_is_reused().await,
        Scenario::DistinctCredentialsLoadSeparately => distinct_credentials_load_separately().await,
        Scenario::SerializedSourcesAreDistinct => serialized_sources_are_distinct().await,
        Scenario::LegacyRecordProject => legacy_record_project().await,
        Scenario::RefreshedRecordKeepsProject => refreshed_record_keeps_project().await,
        Scenario::MissingCredentials => missing_input(MissingInput::Credentials).await,
        Scenario::MissingToken => missing_input(MissingInput::Token).await,
        Scenario::MissingProject => missing_input(MissingInput::Project).await,
        Scenario::UnrelatedRefreshErrorIsNotRetried => {
            unrelated_refresh_error_is_not_retried().await;
        }
        Scenario::StaleTokenIsRefreshed => stale_token_is_refreshed().await,
        Scenario::ExpiringTokenIsRefreshed => expiring_token_is_refreshed().await,
        Scenario::ExpiredFalseWithValidFalseSkipsRefresh => {
            expired_false_with_valid_false_skips_refresh().await;
        }
        Scenario::FallbackFlagsMarkFresh => fallback_flags_mark_fresh().await,
        Scenario::ResolvedAliasServesExplicitProject => {
            resolved_alias_serves_explicit_project().await;
        }
    }
}

async fn project_alias_retained() {
    let executor = Arc::new(FakeExecutor::scripted(
        vec![
            Ok(loaded("token-a", TokenState::Fresh, Some("project-p"))),
            Ok(loaded("token-b", TokenState::Fresh, Some("project-p"))),
        ],
        Vec::new(),
    ));
    let cache = cache_with(Arc::clone(&executor));
    let credentials = credential_text();
    let first = cache
        .get_access_token(Some(&credentials), Some("project-p"))
        .await
        .unwrap();
    assert_eq!(first.token.expose_secret(), "token-a");
    let second = cache
        .get_access_token(Some(&credentials), None)
        .await
        .unwrap();
    assert_eq!(second.token.expose_secret(), "token-b");
    let third = cache
        .get_access_token(Some(&credentials), Some("project-p"))
        .await
        .unwrap();
    assert_eq!(third.token.expose_secret(), "token-a");
    assert_eq!(executor.load_count(), 2);
}

async fn reload_once() {
    let executor = Arc::new(FakeExecutor::scripted(
        vec![
            Ok(loaded("token-old", TokenState::Invalid, Some("project-p"))),
            Ok(loaded("token-new", TokenState::Fresh, Some("project-p"))),
        ],
        vec![RefreshReply::Immediate(Err(reauthentication_error()))],
    ));
    let cache = cache_with(Arc::clone(&executor));
    let credentials = credential_text();
    let access = cache
        .get_access_token(Some(&credentials), Some("project-p"))
        .await
        .unwrap();
    assert_eq!(access.token.expose_secret(), "token-new");
    assert_eq!(access.project, "project-p");
    assert_eq!(executor.load_count(), 2);
    assert_eq!(executor.refresh_count(), 1);
    assert!(cache.leased_identities().is_empty());
}

async fn original_error_survives() {
    let executor = Arc::new(FakeExecutor::scripted(
        vec![
            Ok(loaded("token-old", TokenState::Invalid, Some("project-p"))),
            Err(load_error()),
        ],
        vec![RefreshReply::Immediate(Err(reauthentication_error()))],
    ));
    let cache = cache_with(Arc::clone(&executor));
    let credentials = credential_text();
    let error = cache
        .get_access_token(Some(&credentials), Some("project-p"))
        .await
        .unwrap_err();
    assert!(needs_reauthentication(&error));
    assert_eq!(executor.load_count(), 2);
    assert_eq!(executor.refresh_count(), 1);
}

async fn cached_token_skips_refresh() {
    let executor = Arc::new(FakeExecutor::scripted(
        vec![Ok(loaded("token-a", TokenState::Fresh, Some("project-p")))],
        Vec::new(),
    ));
    let cache = cache_with(Arc::clone(&executor));
    let credentials = credential_text();
    let first = cache
        .get_access_token(Some(&credentials), None)
        .await
        .unwrap();
    let second = cache
        .get_access_token(Some(&credentials), None)
        .await
        .unwrap();
    assert_eq!(first.token.expose_secret(), "token-a");
    assert_eq!(second.token.expose_secret(), "token-a");
    assert_eq!(executor.load_count(), 1);
    assert_eq!(executor.refresh_count(), 0);
}

async fn adc_entry_is_reused() {
    let executor = Arc::new(FakeExecutor::scripted(
        vec![Ok(loaded(
            "token-adc",
            TokenState::Fresh,
            Some("project-p"),
        ))],
        Vec::new(),
    ));
    let cache = cache_with(Arc::clone(&executor));
    let first = cache.get_access_token(None, None).await.unwrap();
    let second = cache.get_access_token(None, None).await.unwrap();
    assert_eq!(first.token.expose_secret(), "token-adc");
    assert_eq!(second.token.expose_secret(), "token-adc");
    assert_eq!(executor.load_count(), 1);
}

async fn distinct_credentials_load_separately() {
    let executor = Arc::new(FakeExecutor::scripted(
        vec![
            Ok(loaded("token-a", TokenState::Fresh, Some("project-p"))),
            Ok(loaded("token-b", TokenState::Fresh, Some("project-p"))),
        ],
        Vec::new(),
    ));
    let cache = cache_with(Arc::clone(&executor));
    let first = ConfiguredCredentials::Text(SecretString::from("credential-a"));
    let second = ConfiguredCredentials::Text(SecretString::from("credential-b"));
    let first_access = cache
        .get_access_token(Some(&first), Some("project-p"))
        .await
        .unwrap();
    let second_access = cache
        .get_access_token(Some(&second), Some("project-p"))
        .await
        .unwrap();
    let again = cache
        .get_access_token(Some(&first), Some("project-p"))
        .await
        .unwrap();
    assert_eq!(first_access.token.expose_secret(), "token-a");
    assert_eq!(second_access.token.expose_secret(), "token-b");
    assert_eq!(again.token.expose_secret(), "token-a");
    assert_eq!(executor.load_count(), 2);
}

async fn serialized_sources_are_distinct() {
    let executor = Arc::new(FakeExecutor::scripted(
        vec![
            Ok(loaded("token-a", TokenState::Fresh, Some("project-a"))),
            Ok(loaded("token-b", TokenState::Fresh, Some("project-b"))),
        ],
        Vec::new(),
    ));
    let cache = cache_with(Arc::clone(&executor));
    let first = service_account("project-a");
    let second = service_account("project-b");
    let first_access = cache.get_access_token(Some(&first), None).await.unwrap();
    let second_access = cache.get_access_token(Some(&second), None).await.unwrap();
    let again = cache.get_access_token(Some(&first), None).await.unwrap();
    assert_eq!(first_access.token.expose_secret(), "token-a");
    assert_eq!(second_access.token.expose_secret(), "token-b");
    assert_eq!(again.token.expose_secret(), "token-a");
    assert_eq!(executor.load_count(), 2);
}

async fn legacy_record_project() {
    let executor = Arc::new(FakeExecutor::scripted(Vec::new(), Vec::new()));
    let cache = cache_with(Arc::clone(&executor));
    let credentials = credential_text();
    let key = CredentialKey::new(Some(&credentials), None);
    cache.store_record(
        key,
        CacheRecord::Legacy {
            credential: Arc::new(legacy_snapshot(Some("quota-project"), Some("own-project"))),
        },
    );
    let access = cache
        .get_access_token(Some(&credentials), None)
        .await
        .unwrap();
    assert_eq!(access.token.expose_secret(), "token-legacy");
    assert_eq!(access.project, "quota-project");
    assert_eq!(executor.load_count(), 0);
}

async fn refreshed_record_keeps_project() {
    let executor = Arc::new(FakeExecutor::scripted(
        vec![Ok(loaded(
            "token-old",
            TokenState::Invalid,
            Some("project-p"),
        ))],
        vec![RefreshReply::Immediate(Ok(snapshot(
            Some("token-new"),
            TokenState::Fresh,
            Some("project-p"),
        )))],
    ));
    let cache = cache_with(Arc::clone(&executor));
    let credentials = credential_text();
    let first = cache
        .get_access_token(Some(&credentials), None)
        .await
        .unwrap();
    assert_eq!(first.token.expose_secret(), "token-new");
    assert_eq!(first.project, "project-p");
    let second = cache
        .get_access_token(Some(&credentials), None)
        .await
        .unwrap();
    assert_eq!(second.token.expose_secret(), "token-new");
    assert_eq!(second.project, "project-p");
    assert_eq!(executor.refresh_count(), 1);
}

#[derive(Clone, Copy)]
enum MissingInput {
    Credentials,
    Token,
    Project,
}

async fn missing_input(expected: MissingInput) {
    let loaded = match expected {
        MissingInput::Credentials => LoadedCredential {
            credential: None,
            project: Some("project-p".to_owned()),
        },
        MissingInput::Token => loaded_from(
            snapshot(None, TokenState::Fresh, Some("project-p")),
            Some("project-p"),
        ),
        MissingInput::Project => {
            loaded_from(snapshot(Some("token-a"), TokenState::Fresh, None), None)
        }
    };
    let executor = Arc::new(FakeExecutor::scripted(vec![Ok(loaded)], Vec::new()));
    let cache = cache_with(Arc::clone(&executor));
    let credentials = credential_text();
    let error = cache
        .get_access_token(Some(&credentials), None)
        .await
        .unwrap_err();
    let matched = match expected {
        MissingInput::Credentials => matches!(error, CredentialCacheError::MissingCredentials),
        MissingInput::Token => matches!(error, CredentialCacheError::MissingToken),
        MissingInput::Project => matches!(error, CredentialCacheError::MissingProject),
    };
    assert!(matched);
}

async fn unrelated_refresh_error_is_not_retried() {
    let executor = Arc::new(FakeExecutor::scripted(
        vec![Ok(loaded(
            "token-old",
            TokenState::Invalid,
            Some("project-p"),
        ))],
        vec![RefreshReply::Immediate(Err(network_error()))],
    ));
    let cache = cache_with(Arc::clone(&executor));
    let credentials = credential_text();
    let error = cache
        .get_access_token(Some(&credentials), Some("project-p"))
        .await
        .unwrap_err();
    assert!(!needs_reauthentication(&error));
    assert!(carries(&error, "timed out"));
    assert_eq!(executor.load_count(), 1);
    assert_eq!(executor.refresh_count(), 1);
}

// a lookup that finds the cached token due for a refresh must refresh through the executor and serve the new token
async fn refreshed_access(loaded: LoadedCredential) -> AccessToken {
    let executor = Arc::new(FakeExecutor::scripted(
        vec![Ok(loaded)],
        vec![RefreshReply::Immediate(Ok(snapshot(
            Some("token-fresh"),
            TokenState::Fresh,
            Some("project-p"),
        )))],
    ));
    let cache = cache_with(Arc::clone(&executor));
    let credentials = credential_text();
    let access = cache
        .get_access_token(Some(&credentials), None)
        .await
        .unwrap();
    assert_eq!(executor.refresh_count(), 1);
    access
}

async fn stale_token_is_refreshed() {
    let access =
        refreshed_access(loaded("token-stale", TokenState::Stale, Some("project-p"))).await;
    assert_eq!(access.token.expose_secret(), "token-fresh");
    assert_eq!(access.project, "project-p");
}

async fn expiring_token_is_refreshed() {
    let expiring = CredentialSnapshot {
        token: Some(SecretString::from("token-old")),
        state: None,
        expires_at: SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(120)),
        expired: None,
        valid: None,
        quota_project_id: None,
        project_id: None,
    };
    let access = refreshed_access(loaded_from(expiring, Some("project-p"))).await;
    assert_eq!(access.token.expose_secret(), "token-fresh");
}

async fn expired_false_with_valid_false_skips_refresh() {
    let executor = Arc::new(FakeExecutor::scripted(
        vec![Ok(LoadedCredential {
            credential: Some(Arc::new(CredentialSnapshot {
                token: Some(SecretString::from("token-lax")),
                state: None,
                expires_at: None,
                expired: Some(false),
                valid: Some(false),
                quota_project_id: None,
                project_id: None,
            })),
            project: Some("project-p".to_owned()),
        })],
        Vec::new(),
    ));
    let cache = cache_with(Arc::clone(&executor));
    let credentials = credential_text();
    let access = cache
        .get_access_token(Some(&credentials), None)
        .await
        .unwrap();
    assert_eq!(access.token.expose_secret(), "token-lax");
    assert_eq!(access.project, "project-p");
    assert_eq!(executor.refresh_count(), 0);
}

async fn fallback_flags_mark_fresh() {
    let executor = Arc::new(FakeExecutor::scripted(
        vec![Ok(LoadedCredential {
            credential: Some(Arc::new(CredentialSnapshot {
                token: Some(SecretString::from("token-fallback")),
                state: None,
                expires_at: None,
                expired: Some(false),
                valid: Some(true),
                quota_project_id: None,
                project_id: None,
            })),
            project: Some("project-p".to_owned()),
        })],
        Vec::new(),
    ));
    let cache = cache_with(Arc::clone(&executor));
    let credentials = credential_text();
    let access = cache
        .get_access_token(Some(&credentials), None)
        .await
        .unwrap();
    assert_eq!(access.token.expose_secret(), "token-fallback");
    assert_eq!(access.project, "project-p");
    assert_eq!(executor.refresh_count(), 0);
}

fn gated_executor(
    started: &Arc<Notify>,
    gate: &Arc<Notify>,
    first_token: &str,
    second_token: &str,
) -> Arc<FakeExecutor> {
    Arc::new(FakeExecutor::scripted(
        vec![Ok(loaded(
            "token-old",
            TokenState::Invalid,
            Some("project-p"),
        ))],
        vec![
            RefreshReply::Gated {
                started: Arc::clone(started),
                gate: Arc::clone(gate),
                reply: Ok(snapshot(
                    Some(first_token),
                    TokenState::Fresh,
                    Some("project-p"),
                )),
            },
            RefreshReply::Immediate(Ok(snapshot(
                Some(second_token),
                TokenState::Fresh,
                Some("project-p"),
            ))),
        ],
    ))
}

#[tokio::test]
async fn cancellation_releases_lease() {
    let started = Arc::new(Notify::new());
    let gate = Arc::new(Notify::new());
    let executor = gated_executor(&started, &gate, "token-refreshed", "token-second");
    let cache = Arc::new(cache_with(Arc::clone(&executor)));
    let credentials = credential_text();
    let key = CredentialKey::new(Some(&credentials), Some("project-p"));
    let mut cancelled = Box::pin(cache.get_access_token(Some(&credentials), Some("project-p")));
    assert!(poll_once(cancelled.as_mut()).is_pending());
    assert_eq!(cache.leased_identities(), vec![key]);
    drop(cancelled);
    assert!(cache.leased_identities().is_empty());
    let access = cache
        .get_access_token(Some(&credentials), Some("project-p"))
        .await
        .unwrap();
    assert_eq!(access.token.expose_secret(), "token-second");
    assert!(cache.leased_identities().is_empty());
}

#[tokio::test]
async fn a_waiting_caller_keeps_the_lease_entry() {
    let started = Arc::new(Notify::new());
    let gate = Arc::new(Notify::new());
    let executor = gated_executor(&started, &gate, "token-first", "token-second");
    let cache = Arc::new(cache_with(Arc::clone(&executor)));
    let credentials = credential_text();
    let key = CredentialKey::new(Some(&credentials), Some("project-p"));
    let mut first = Box::pin(cache.get_access_token(Some(&credentials), Some("project-p")));
    assert!(poll_once(first.as_mut()).is_pending());
    let mut second = Box::pin(cache.get_access_token(Some(&credentials), Some("project-p")));
    assert!(poll_once(second.as_mut()).is_pending());
    assert_eq!(cache.leased_identities(), vec![key.clone()]);
    drop(first);
    assert_eq!(cache.leased_identities(), vec![key]);
    let access = second.await.unwrap();
    assert_eq!(access.token.expose_secret(), "token-second");
    assert_eq!(executor.refresh_count(), 2);
    assert!(cache.leased_identities().is_empty());
}

// a reauthentication clears the entry while a waiter still holds the old record, so the refresh must keep its project
#[tokio::test]
async fn a_forgotten_entry_keeps_the_project_it_resolved() {
    let started = Arc::new(Notify::new());
    let gate = Arc::new(Notify::new());
    let executor = Arc::new(FakeExecutor::scripted(
        vec![Ok(loaded(
            "token-stale",
            TokenState::Invalid,
            Some("quota-project"),
        ))],
        vec![
            RefreshReply::Gated {
                started: Arc::clone(&started),
                gate: Arc::clone(&gate),
                reply: Ok(snapshot(Some("token-first"), TokenState::Fresh, None)),
            },
            RefreshReply::Immediate(Ok(snapshot(Some("token-second"), TokenState::Fresh, None))),
        ],
    ));
    let cache = Arc::new(cache_with(Arc::clone(&executor)));
    let credentials = credential_text();
    let key = CredentialKey::new(Some(&credentials), None);
    let mut first = Box::pin(cache.get_access_token(Some(&credentials), None));
    assert!(poll_once(first.as_mut()).is_pending());
    let mut second = Box::pin(cache.get_access_token(Some(&credentials), None));
    assert!(poll_once(second.as_mut()).is_pending());
    gate.notify_one();
    first.await.unwrap();
    cache.forget_record(&key);
    let second_access = second.await.unwrap();
    assert_eq!(second_access.token.expose_secret(), "token-second");
    let third = cache
        .get_access_token(Some(&credentials), None)
        .await
        .unwrap();
    assert_eq!(third.token.expose_secret(), "token-second");
    assert_eq!(third.project, "quota-project");
    assert_eq!(executor.refresh_count(), 2);
}

#[tokio::test]
async fn credentials_are_redacted() {
    let executor = Arc::new(FakeExecutor::scripted(
        vec![Ok(loaded(
            SECRET_TOKEN,
            TokenState::Fresh,
            Some("project-p"),
        ))],
        Vec::new(),
    ));
    let cache = cache_with(Arc::clone(&executor));
    let credentials = ConfiguredCredentials::Text(SecretString::from(SECRET_CREDENTIAL));
    let access = cache
        .get_access_token(Some(&credentials), None)
        .await
        .unwrap();
    let key = CredentialKey::new(Some(&credentials), None);
    let record = cache.cached_record(&key).unwrap();
    assert!(!format!("{cache:?}").contains(SECRET_TOKEN));
    assert!(!format!("{key:?}").contains(SECRET_CREDENTIAL));
    assert!(!format!("{record:?}").contains(SECRET_TOKEN));
    assert!(!format!("{record:?}").contains(SECRET_CREDENTIAL));
    assert!(!format!("{access:?}").contains(SECRET_TOKEN));
    assert!(format!("{cache:?}").contains("records: 2"));
    assert_eq!(access.token.expose_secret(), SECRET_TOKEN);
}

async fn resolved_alias_serves_explicit_project() {
    let executor = Arc::new(FakeExecutor::scripted(
        vec![Ok(loaded("token-a", TokenState::Fresh, Some("project-p")))],
        Vec::new(),
    ));
    let cache = cache_with(Arc::clone(&executor));
    let credentials = credential_text();
    let unresolved = cache
        .get_access_token(Some(&credentials), None)
        .await
        .unwrap();
    assert_eq!(unresolved.token.expose_secret(), "token-a");
    assert_eq!(unresolved.project, "project-p");
    let explicit = cache
        .get_access_token(Some(&credentials), Some("project-p"))
        .await
        .unwrap();
    assert_eq!(explicit.token.expose_secret(), "token-a");
    assert_eq!(explicit.project, "project-p");
    assert_eq!(executor.load_count(), 1);
}

#[tokio::test]
async fn a_retried_refresh_keeps_the_first_error() {
    let executor = Arc::new(FakeExecutor::repeating_reauth());
    let cache = cache_with(Arc::clone(&executor));
    let credentials = credential_text();
    let error = cache
        .get_access_token(Some(&credentials), Some("project-p"))
        .await
        .unwrap_err();
    assert!(needs_reauthentication(&error));
    assert_eq!(executor.load_count(), 2);
    assert_eq!(executor.refresh_count(), 2);
}
