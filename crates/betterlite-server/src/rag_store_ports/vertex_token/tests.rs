#![cfg(test)]

use std::collections::VecDeque;
use std::error::Error as _;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_core::rag::vertex_ingestion::ports::VertexTokenPort as _;
use betterlite_vertex::credential_sources::error::CredentialSourceError;
use betterlite_vertex::credential_sources::sdk::CredentialClock;
use betterlite_vertex::token_cache::CredentialCache;
use betterlite_vertex::token_cache::error::CredentialCacheError;
use betterlite_vertex::token_cache::executor::{CredentialExecutor, LoadedCredential};
use betterlite_vertex::token_cache::record::CredentialSnapshot;
use betterlite_vertex::token_cache::state::TokenState;
use secrecy::{ExposeSecret as _, SecretString};

use super::VertexTokenSource;

struct FixedClock(SystemTime);

impl CredentialClock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

struct FakeExecutor {
    loads: Mutex<VecDeque<Result<LoadedCredential, CredentialCacheError>>>,
    saw_credentials: Mutex<Vec<bool>>,
}

impl FakeExecutor {
    fn scripted(loads: Vec<Result<LoadedCredential, CredentialCacheError>>) -> Self {
        Self {
            loads: Mutex::new(VecDeque::from(loads)),
            saw_credentials: Mutex::new(Vec::new()),
        }
    }

    fn seen(&self) -> Vec<bool> {
        self.saw_credentials
            .lock()
            .expect("the fixture lock is not poisoned")
            .clone()
    }
}

impl CredentialExecutor for FakeExecutor {
    fn load<'borrow>(
        &'borrow self,
        credentials: Option<&'borrow ConfiguredCredentials>,
        _project: Option<&'borrow str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<LoadedCredential, CredentialCacheError>> + Send + 'borrow>,
    > {
        self.saw_credentials
            .lock()
            .expect("the fixture lock is not poisoned")
            .push(credentials.is_some());
        let next = self
            .loads
            .lock()
            .expect("the fixture lock is not poisoned")
            .pop_front();
        let reply = next.unwrap_or(Err(CredentialCacheError::MissingCredentials));
        Box::pin(async move { reply })
    }

    fn refresh<'borrow>(
        &'borrow self,
        _credential: &'borrow CredentialSnapshot,
    ) -> Pin<
        Box<dyn Future<Output = Result<CredentialSnapshot, CredentialCacheError>> + Send + 'borrow>,
    > {
        Box::pin(async { Err(CredentialCacheError::MissingToken) })
    }
}

fn loaded(token: &str, project: &str) -> LoadedCredential {
    LoadedCredential {
        credential: Some(Arc::new(CredentialSnapshot {
            token: Some(SecretString::from(token)),
            state: Some(TokenState::Fresh),
            expires_at: None,
            expired: None,
            valid: None,
            quota_project_id: None,
            project_id: Some(project.to_owned()),
        })),
        project: Some(project.to_owned()),
    }
}

fn make(executor: Arc<FakeExecutor>) -> VertexTokenSource {
    let clock: Arc<dyn CredentialClock> = Arc::new(FixedClock(SystemTime::UNIX_EPOCH));
    let executor_port: Arc<dyn CredentialExecutor> = executor;
    VertexTokenSource {
        cache: Arc::new(CredentialCache::new(clock, executor_port)),
    }
}

#[tokio::test]
async fn the_token_and_the_resolved_project_are_returned() {
    let executor = Arc::new(FakeExecutor::scripted(vec![Ok(loaded(
        "token-1",
        "project-1",
    ))]));
    let port = make(Arc::clone(&executor));
    let (token, project) = port
        .access_token(None, None)
        .await
        .expect("the token resolves");
    assert_eq!(token.expose_secret(), "token-1");
    assert_eq!(project, "project-1");
    assert_eq!(executor.seen(), vec![false]);
}

#[tokio::test]
async fn explicit_credentials_are_loaded_as_such() {
    let executor = Arc::new(FakeExecutor::scripted(vec![Ok(loaded(
        "token-2",
        "project-2",
    ))]));
    let port = make(Arc::clone(&executor));
    let credentials = ConfiguredCredentials::Text(SecretString::from("{}"));
    let (token, project) = port
        .access_token(Some(&credentials), None)
        .await
        .expect("the token resolves");
    assert_eq!(token.expose_secret(), "token-2");
    assert_eq!(project, "project-2");
    assert_eq!(executor.seen(), vec![true]);
}

#[tokio::test]
async fn application_defaults_are_loaded_without_credentials() {
    let executor = Arc::new(FakeExecutor::scripted(vec![Ok(loaded(
        "token-3",
        "project-3",
    ))]));
    let port = make(Arc::clone(&executor));
    let (token, _project) = port
        .access_token(None, None)
        .await
        .expect("the token resolves");
    assert_eq!(token.expose_secret(), "token-3");
    assert_eq!(executor.seen(), vec![false]);
}

#[tokio::test]
async fn a_load_failure_is_a_port_error() {
    let executor = Arc::new(FakeExecutor::scripted(vec![Err(
        CredentialCacheError::Load(CredentialSourceError::MissingProject),
    )]));
    let port = make(executor);
    let error = port
        .access_token(None, None)
        .await
        .expect_err("the load fails");
    assert_eq!(
        error.to_string(),
        "the vertex credential could not be loaded"
    );
    let native = error.source().expect("the native error is the source");
    assert!(matches!(
        native.downcast_ref::<CredentialCacheError>(),
        Some(CredentialCacheError::Load(_))
    ));
}

#[tokio::test]
async fn the_caller_project_is_returned() {
    let executor = Arc::new(FakeExecutor::scripted(vec![Ok(loaded(
        "token-5",
        "credential-project",
    ))]));
    let port = make(executor);
    let (_token, project) = port
        .access_token(None, Some("caller-project"))
        .await
        .expect("the token resolves");
    assert_eq!(project, "caller-project");
}

#[tokio::test]
async fn the_token_never_shows_in_debug() {
    let executor = Arc::new(FakeExecutor::scripted(vec![Ok(loaded(
        "token-secret",
        "project-6",
    ))]));
    let port = make(executor);
    assert_eq!(format!("{port:?}"), "VertexTokenSource { .. }");
    let (token, _project) = port
        .access_token(None, None)
        .await
        .expect("the token resolves");
    assert_eq!(token.expose_secret(), "token-secret");
    assert!(!format!("{port:?}").contains("token-secret"));
}
