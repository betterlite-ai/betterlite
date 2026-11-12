pub mod error;
pub mod executor;
pub mod key;
pub mod lease;
pub mod probe;
pub mod record;
pub mod state;
mod tests;

use std::collections::BTreeMap;
use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;

use crate::credential_sources::sdk::CredentialClock;
use error::{CredentialCacheError, needs_reauthentication};
use executor::CredentialExecutor;
use key::CredentialKey;
use lease::{LeaseMap, RefreshLease};
use probe::AccessToken;
use record::{CacheRecord, CredentialSnapshot, unpack_cached_credentials};
use state::{TokenState, token_state};

// VertexBase's credential state: the cached records, their refresh bookkeeping and the injected clock and executor
pub struct CredentialCache {
    records: Mutex<BTreeMap<CredentialKey, CacheRecord>>,
    leases: Arc<LeaseMap>,
    clock: Arc<dyn CredentialClock>,
    executor: Arc<dyn CredentialExecutor>,
}

enum RefreshFailure {
    Reauthenticate(CredentialCacheError),
    Failed(CredentialCacheError),
}

impl CredentialCache {
    #[must_use]
    pub fn new(clock: Arc<dyn CredentialClock>, executor: Arc<dyn CredentialExecutor>) -> Self {
        Self {
            records: Mutex::new(BTreeMap::new()),
            leases: Arc::new(LeaseMap::default()),
            clock,
            executor,
        }
    }

    // VertexBase.get_access_token: the synchronous lookup with its one-shot reauthentication branch
    pub async fn get_access_token(
        &self,
        credentials: Option<&ConfiguredCredentials>,
        project: Option<&str>,
    ) -> Result<AccessToken, CredentialCacheError> {
        self.access_token(
            credentials,
            project,
            &CredentialKey::new(credentials, project),
            false,
        )
        .await
    }

    async fn access_token(
        &self,
        credentials: Option<&ConfiguredCredentials>,
        project: Option<&str>,
        key: &CredentialKey,
        retry_reauth: bool,
    ) -> Result<AccessToken, CredentialCacheError> {
        let record = self.cached_or_loaded(credentials, project, key).await?;
        let (cached, cached_project) = unpack_cached_credentials(Some(&record));
        let held = Arc::clone(cached.ok_or(CredentialCacheError::MissingCredentials)?);
        let mut resolved_project = project.map(str::to_owned);
        if resolved_project.is_none()
            && let Some(record_project) = cached_project
        {
            // LiteLLM fills the resolved alias only when it is absent, so an existing one is kept
            let resolved_key = CredentialKey {
                credential: key.credential.clone(),
                project: Some(record_project.to_owned()),
            };
            self.store_record_if_absent(resolved_key, &record);
            resolved_project = Some(record_project.to_owned());
        }
        let credential = match self.refresh_if_invalid(key, held, retry_reauth).await {
            Ok(credential) => credential,
            Err(RefreshFailure::Reauthenticate(error)) => {
                return self
                    .reauthenticate(credentials, resolved_project.as_deref(), key, error)
                    .await;
            }
            Err(RefreshFailure::Failed(error)) => return Err(error),
        };
        let token = credential
            .token
            .clone()
            .ok_or(CredentialCacheError::MissingToken)?;
        let resolved = resolved_project.ok_or(CredentialCacheError::MissingProject)?;
        Ok(AccessToken {
            token,
            project: resolved,
        })
    }

    async fn cached_or_loaded(
        &self,
        credentials: Option<&ConfiguredCredentials>,
        project: Option<&str>,
        key: &CredentialKey,
    ) -> Result<CacheRecord, CredentialCacheError> {
        if let Some(record) = self.cached_record(key) {
            return Ok(record);
        }
        self.load_and_cache(credentials, project, key).await
    }

    // LiteLLM's _load_and_cache_credentials: load through load_auth and cache what it resolved
    pub async fn load_and_cache(
        &self,
        credentials: Option<&ConfiguredCredentials>,
        project: Option<&str>,
        key: &CredentialKey,
    ) -> Result<CacheRecord, CredentialCacheError> {
        let loaded = self.executor.load(credentials, project).await?;
        let record = CacheRecord::Paired {
            credential: loaded
                .credential
                .ok_or(CredentialCacheError::MissingCredentials)?,
            project: loaded.project,
        };
        self.store_record(key.clone(), record.clone());
        Ok(record)
    }

    async fn refresh_if_invalid(
        &self,
        key: &CredentialKey,
        credential: Arc<CredentialSnapshot>,
        retry_reauth: bool,
    ) -> Result<Arc<CredentialSnapshot>, RefreshFailure> {
        if token_state(&credential, self.clock.now()) != TokenState::Invalid {
            return Ok(credential);
        }
        let lease = self
            .acquire_refresh_lease(key)
            .await
            .map_err(RefreshFailure::Failed)?;
        let current = self
            .cached_record(key)
            .unwrap_or_else(|| CacheRecord::Paired {
                credential: Arc::clone(&credential),
                project: None,
            });
        let current_credential = Arc::clone(current.credential());
        // LiteLLM rechecks after taking its refresh lock: another caller may have refreshed already
        if token_state(&current_credential, self.clock.now()) != TokenState::Invalid {
            drop(lease);
            return Ok(current_credential);
        }
        let refreshed = match self.executor.refresh(&current_credential).await {
            Ok(snapshot) => Arc::new(snapshot),
            Err(error) => {
                drop(lease);
                return Err(if needs_reauthentication(&error) && !retry_reauth {
                    RefreshFailure::Reauthenticate(error)
                } else {
                    RefreshFailure::Failed(error)
                });
            }
        };
        self.store_record(key.clone(), current.with_credential(Arc::clone(&refreshed)));
        drop(lease);
        Ok(refreshed)
    }

    // LiteLLM's _handle_reauthentication: clear the entry, retry once, and keep the first failure
    async fn reauthenticate(
        &self,
        credentials: Option<&ConfiguredCredentials>,
        project: Option<&str>,
        key: &CredentialKey,
        original: CredentialCacheError,
    ) -> Result<AccessToken, CredentialCacheError> {
        self.forget_record(key);
        let retry_key = CredentialKey::new(credentials, project);
        match Box::pin(self.access_token(credentials, project, &retry_key, true)).await {
            Ok(access) => Ok(access),
            Err(_retry) => Err(original),
        }
    }

    #[must_use]
    pub fn cached_record(&self, key: &CredentialKey) -> Option<CacheRecord> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    pub fn store_record(&self, key: CredentialKey, record: CacheRecord) {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, record);
    }

    pub fn store_record_if_absent(&self, key: CredentialKey, record: &CacheRecord) {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(key)
            .or_insert_with(|| record.clone());
    }

    pub fn forget_record(&self, key: &CredentialKey) {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
    }

    // the request-scoped refresh lease; LiteLLM's _acquire_async_refresh_lock and release pair
    pub async fn acquire_refresh_lease(
        &self,
        key: &CredentialKey,
    ) -> Result<RefreshLease, CredentialCacheError> {
        let hold = self.leases.hold(key);
        let semaphore = Arc::clone(&hold.semaphore);
        let permit = semaphore
            .acquire_owned()
            .await
            .map_err(|_error| CredentialCacheError::LeaseClosed)?;
        Ok(RefreshLease {
            _permit: permit,
            _hold: hold,
        })
    }

    #[must_use]
    pub fn leased_identities(&self) -> Vec<CredentialKey> {
        let entries = self
            .leases
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        entries.keys().cloned().collect()
    }
}

impl Debug for CredentialCache {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let records = self
            .records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        let leases = self.leased_identities().len();
        f.debug_struct("CredentialCache")
            .field("records", &records)
            .field("leases", &leases)
            .finish_non_exhaustive()
    }
}
