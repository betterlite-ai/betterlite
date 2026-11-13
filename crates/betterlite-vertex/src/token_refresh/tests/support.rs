use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use google_cloud_auth::errors::CredentialsError;
use secrecy::SecretString;
use tokio::sync::Notify;

use crate::credential_sources::error::CredentialSourceError;
use crate::credential_sources::sdk::CredentialClock;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::executor::{CredentialExecutor, LoadedCredential};
use crate::token_cache::record::CredentialSnapshot;
use crate::token_cache::state::TokenState;

pub(super) struct ControlledClock(Mutex<SystemTime>);

impl ControlledClock {
    pub(super) fn at(now: SystemTime) -> Self {
        Self(Mutex::new(now))
    }

    pub(super) fn set(&self, now: SystemTime) {
        *self.0.lock().unwrap() = now;
    }
}

impl CredentialClock for ControlledClock {
    fn now(&self) -> SystemTime {
        *self.0.lock().unwrap()
    }
}

pub(super) enum RefreshReply {
    Immediate(Result<CredentialSnapshot, CredentialCacheError>),
    Gated {
        started: Arc<Notify>,
        gate: Arc<Notify>,
        reply: Result<CredentialSnapshot, CredentialCacheError>,
    },
}

pub(super) struct ControlledExecutor {
    loads: Mutex<VecDeque<Result<LoadedCredential, CredentialCacheError>>>,
    refreshes: Mutex<VecDeque<RefreshReply>>,
    load_calls: AtomicUsize,
    refresh_calls: AtomicUsize,
    targets: Mutex<Vec<Option<SecretString>>>,
}

impl ControlledExecutor {
    pub(super) fn scripted(
        loads: Vec<Result<LoadedCredential, CredentialCacheError>>,
        refreshes: Vec<RefreshReply>,
    ) -> Self {
        Self {
            loads: Mutex::new(VecDeque::from(loads)),
            refreshes: Mutex::new(VecDeque::from(refreshes)),
            load_calls: AtomicUsize::new(0),
            refresh_calls: AtomicUsize::new(0),
            targets: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn load_count(&self) -> usize {
        self.load_calls.load(Ordering::SeqCst)
    }

    pub(super) fn refresh_count(&self) -> usize {
        self.refresh_calls.load(Ordering::SeqCst)
    }

    pub(super) fn refresh_targets(&self) -> Vec<Option<SecretString>> {
        self.targets.lock().unwrap().clone()
    }
}

async fn answer(next: Option<RefreshReply>) -> Result<CredentialSnapshot, CredentialCacheError> {
    match next {
        Some(RefreshReply::Immediate(reply)) => reply,
        Some(RefreshReply::Gated {
            started,
            gate,
            reply,
        }) => {
            started.notify_one();
            gate.notified().await;
            reply
        }
        None => Err(network_error()),
    }
}

impl CredentialExecutor for ControlledExecutor {
    fn load<'borrow>(
        &'borrow self,
        _credentials: Option<&'borrow ConfiguredCredentials>,
        _project: Option<&'borrow str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<LoadedCredential, CredentialCacheError>> + Send + 'borrow>,
    > {
        self.load_calls.fetch_add(1, Ordering::SeqCst);
        let next = self.loads.lock().unwrap().pop_front();
        Box::pin(async move { next.unwrap_or_else(|| Err(load_error())) })
    }

    fn refresh<'borrow>(
        &'borrow self,
        credential: &'borrow CredentialSnapshot,
    ) -> Pin<
        Box<dyn Future<Output = Result<CredentialSnapshot, CredentialCacheError>> + Send + 'borrow>,
    > {
        self.refresh_calls.fetch_add(1, Ordering::SeqCst);
        self.targets.lock().unwrap().push(credential.token.clone());
        let next = self.refreshes.lock().unwrap().pop_front();
        Box::pin(answer(next))
    }
}

pub(super) fn snapshot(
    token: Option<&str>,
    state: TokenState,
    project: Option<&str>,
) -> CredentialSnapshot {
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

pub(super) fn expiring(token: &str, expires_at: SystemTime) -> CredentialSnapshot {
    CredentialSnapshot {
        token: Some(SecretString::from(token)),
        state: None,
        expires_at: Some(expires_at),
        expired: None,
        valid: None,
        quota_project_id: None,
        project_id: None,
    }
}

pub(super) fn loaded(token: &str, state: TokenState, project: Option<&str>) -> LoadedCredential {
    LoadedCredential {
        credential: Some(Arc::new(snapshot(Some(token), state, project))),
        project: project.map(str::to_owned),
    }
}

pub(super) fn reauthentication_error() -> CredentialCacheError {
    CredentialCacheError::Refresh(CredentialSourceError::Refresh(CredentialsError::from_msg(
        false,
        "Reauthentication is needed. Please run `gcloud auth application-default login`",
    )))
}

pub(super) fn network_error() -> CredentialCacheError {
    CredentialCacheError::Refresh(CredentialSourceError::Refresh(CredentialsError::from_msg(
        false,
        "the token endpoint timed out",
    )))
}

fn load_error() -> CredentialCacheError {
    CredentialCacheError::Load(CredentialSourceError::MissingProject)
}
