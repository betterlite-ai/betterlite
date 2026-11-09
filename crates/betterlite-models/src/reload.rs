use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::catalog::{BundledCatalog, CatalogError, ModelCatalog};
use crate::fetch_policy::client::{CatalogClient, CatalogRequest};
use crate::fetch_policy::outcome::{Retryable, Unavailable};
use crate::fetch_policy::retry::{CatalogSleeper, JitterSource};
use crate::source::{
    CatalogClock, CatalogProvenance, CatalogSnapshot, CatalogSource, CatalogStore, SourceKind,
};

pub const DEFAULT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);
pub const DEFAULT_MAX_ATTEMPTS: u32 = 3;

pub trait RuntimeRegistrations: Send + Sync {
    fn reapply(&self, catalog: &ModelCatalog);
}

// A process without runtime registrations still adopts catalogs on the same path as one with them
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoRuntimeRegistrations;

impl RuntimeRegistrations for NoRuntimeRegistrations {
    fn reapply(&self, _catalog: &ModelCatalog) {}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FetchLimits {
    pub timeout: Duration,
    pub max_attempts: u32,
}

impl Default for FetchLimits {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_ATTEMPT_TIMEOUT,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
        }
    }
}

pub(crate) enum BundledSource<'source> {
    Local,
    Fallback { url: &'source str, reason: String },
}

pub struct CatalogService<F = fn() -> Result<Vec<u8>, CatalogError>> {
    pub source: CatalogSource,
    pub bundled: Mutex<BundledCatalog<F>>,
    pub clock: CatalogClock,
    pub registrations: Arc<dyn RuntimeRegistrations>,
    pub current: CatalogStore,
}

impl<F: Fn() -> Result<Vec<u8>, CatalogError> + Send + Sync> CatalogService<F> {
    fn bundled_catalog(&self) -> Arc<ModelCatalog> {
        match self.bundled.lock() {
            Ok(mut bundled) => bundled.get(),
            Err(poisoned) => poisoned.into_inner().get(),
        }
    }

    fn bundled_snapshot(
        &self,
        source: BundledSource<'_>,
        loaded_at: DateTime<Utc>,
    ) -> Arc<CatalogSnapshot> {
        let catalog = self.bundled_catalog();
        let provenance = match source {
            BundledSource::Local => CatalogProvenance {
                kind: SourceKind::Local,
                url: None,
                is_env_forced: true,
                fallback_reason: None,
                loaded_at: Some(loaded_at),
                source_revision: catalog.revision.clone(),
                etag: None,
            },
            BundledSource::Fallback { url, reason } => CatalogProvenance {
                kind: SourceKind::Local,
                url: Some(url.to_owned()),
                is_env_forced: false,
                fallback_reason: Some(reason),
                loaded_at: Some(loaded_at),
                source_revision: catalog.revision.clone(),
                etag: None,
            },
        };
        Arc::new(CatalogSnapshot {
            catalog,
            provenance,
        })
    }

    fn adopt_bundled(
        &self,
        source: BundledSource<'_>,
        loaded_at: DateTime<Utc>,
    ) -> Arc<CatalogSnapshot> {
        let snapshot = self.bundled_snapshot(source, loaded_at);
        self.current.store(Arc::clone(&snapshot));
        snapshot
    }

    pub async fn reload<C, S, J>(
        &self,
        limits: FetchLimits,
        client: &C,
        sleeper: &S,
        jitter: &mut J,
    ) -> ReloadOutcome
    where
        C: CatalogClient,
        S: CatalogSleeper,
        J: JitterSource,
    {
        let CatalogSource::Remote { url } = &self.source else {
            let snapshot = self.adopt_bundled(BundledSource::Local, (self.clock)());
            return ReloadOutcome::Reloaded(snapshot);
        };
        let fetch = refresh::ReloadFetch {
            limits,
            client,
            sleeper,
            jitter,
        };
        refresh::reload_remote(self, url, fetch).await
    }

    pub async fn startup<C, S, J>(
        &self,
        limits: FetchLimits,
        client: Arc<C>,
        sleeper: Arc<S>,
        jitter: J,
    ) -> StartupOutcome<C, S, J>
    where
        C: CatalogClient + Send + Sync + 'static,
        S: CatalogSleeper + 'static,
        J: JitterSource + Send + 'static,
    {
        match &self.source {
            CatalogSource::Local => StartupOutcome {
                snapshot: self.adopt_bundled(BundledSource::Local, (self.clock)()),
                retry: None,
            },
            CatalogSource::Remote { url } => {
                let deps = startup::RetryDeps {
                    client,
                    sleeper,
                    jitter,
                };
                startup::remote(self, url, limits, deps).await
            }
        }
    }
}

pub struct StartupOutcome<C, S, J> {
    pub snapshot: Arc<CatalogSnapshot>,
    pub retry: Option<RetryTask<C, S, J>>,
}

pub struct RetryTask<C, S, J> {
    request: CatalogRequest,
    first_outcome: Retryable,
    limits: FetchLimits,
    client: Arc<C>,
    sleeper: Arc<S>,
    jitter: J,
}

#[derive(Clone, Debug)]
pub enum ReloadOutcome {
    Reloaded(Arc<CatalogSnapshot>),
    Unavailable(Unavailable),
}

#[derive(Clone, Debug)]
pub enum RetryOutcome {
    Adopted(Arc<CatalogSnapshot>),
    Kept,
}

mod refresh;
mod retry;
mod startup;
mod tests;
