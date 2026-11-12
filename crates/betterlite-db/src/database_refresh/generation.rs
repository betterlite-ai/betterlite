use std::convert::Infallible;
use std::error::Error;
use std::mem;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinSet;

use crate::database_tokens::endpoint::IAMEndpoint;
use crate::database_tokens::expiry::TokenAuthKind;

use super::access::RefreshTrigger;
use super::endpoint::DatabaseUrlSlot;
use super::handle::{PoolHandle as _, PoolProvider};
use super::tracker::DrainTracker;

pub struct GenerationOptions {
    pub iam_token_db_auth: bool,
    pub token_auth: Option<TokenAuthKind>,
    pub db_url_env_var: String,
    pub url: DatabaseUrlSlot,
    pub iam_endpoint: Option<IAMEndpoint>,
    pub recreate_uses_datasource: bool,
    pub log_prefix: String,
}

impl Default for GenerationOptions {
    fn default() -> Self {
        Self {
            iam_token_db_auth: false,
            token_auth: None,
            db_url_env_var: "DATABASE_URL".to_owned(),
            url: DatabaseUrlSlot::default(),
            iam_endpoint: None,
            recreate_uses_datasource: false,
            log_prefix: String::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GenerationError {
    #[error("a token-auth pool generation requires an injected refresh port")]
    RefreshPortRequired,
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectError<PoolError> {
    #[error("no database pool handle is selected")]
    NotConnected,
    #[error("the database pool could not connect")]
    Connect {
        #[source]
        source: PoolError,
    },
}

pub(crate) struct ActivePool<H> {
    pub(crate) handle: Arc<H>,
    pub(crate) tracker: Option<Arc<DrainTracker>>,
}

impl<H> Clone for ActivePool<H> {
    fn clone(&self) -> Self {
        Self {
            handle: Arc::clone(&self.handle),
            tracker: self.tracker.as_ref().map(Arc::clone),
        }
    }
}

pub struct Observation<H> {
    pub generation: u64,
    pub handle: Option<Arc<H>>,
    pub connected: bool,
}

pub struct PoolGeneration<P, T, C = Infallible>
where
    P: PoolProvider,
    T: RefreshTrigger,
    C: Error + Send + Sync + 'static,
{
    pub(crate) options: GenerationOptions,
    pub(crate) replacement_lock: AsyncMutex<()>,
    pub(crate) last_refresh_time: Mutex<Option<DateTime<Utc>>>,
    pub(crate) provider: Arc<P>,
    pub(crate) refresh: Option<Arc<T>>,
    pub(crate) on_replaced: Option<Arc<dyn Fn() -> Result<(), C> + Send + Sync>>,
    pub(crate) current: Mutex<Option<ActivePool<P::Handle>>>,
    pub(crate) generation: AtomicU64,
    pub(crate) retirements: Mutex<JoinSet<()>>,
    pub(crate) refreshes: Mutex<JoinSet<()>>,
}

impl<P, T, C> PoolGeneration<P, T, C>
where
    P: PoolProvider,
    T: RefreshTrigger,
    C: Error + Send + Sync + 'static,
{
    pub fn new(
        provider: Arc<P>,
        options: GenerationOptions,
        handle: Option<Arc<P::Handle>>,
        refresh: Option<Arc<T>>,
        on_replaced: Option<Arc<dyn Fn() -> Result<(), C> + Send + Sync>>,
    ) -> Result<Self, GenerationError> {
        let token_auth = options
            .token_auth
            .or_else(|| options.iam_token_db_auth.then_some(TokenAuthKind::Rds));
        if token_auth.is_some() && refresh.is_none() {
            return Err(GenerationError::RefreshPortRequired);
        }
        let log_prefix = if options.log_prefix.is_empty() {
            String::new()
        } else {
            format!("{} ", options.log_prefix)
        };
        let normalized = GenerationOptions {
            iam_token_db_auth: options.iam_token_db_auth,
            token_auth,
            db_url_env_var: options.db_url_env_var,
            url: options.url,
            iam_endpoint: options.iam_endpoint,
            recreate_uses_datasource: options.recreate_uses_datasource,
            log_prefix,
        };
        let initial = handle.map(|initial_handle| ActivePool {
            handle: initial_handle,
            tracker: None,
        });
        Ok(Self {
            options: normalized,
            replacement_lock: AsyncMutex::new(()),
            last_refresh_time: Mutex::new(None),
            provider,
            refresh,
            on_replaced,
            current: Mutex::new(initial),
            generation: AtomicU64::new(0),
            retirements: Mutex::new(JoinSet::new()),
            refreshes: Mutex::new(JoinSet::new()),
        })
    }

    #[must_use]
    pub const fn token_label(&self) -> &'static str {
        match self.options.token_auth {
            Some(TokenAuthKind::Rds) => "RDS IAM token",
            Some(TokenAuthKind::Azure) => "Azure Entra token",
            None => "database token",
        }
    }

    #[must_use]
    pub const fn iam_token_db_auth(&self) -> bool {
        match self.options.token_auth {
            Some(TokenAuthKind::Rds | TokenAuthKind::Azure) => true,
            None => false,
        }
    }

    pub fn record_refresh(&self, at: DateTime<Utc>) {
        match self.last_refresh_time.lock() {
            Ok(mut slot) => *slot = Some(at),
            Err(poisoned) => *poisoned.into_inner() = Some(at),
        }
    }

    #[must_use]
    pub fn observe(&self) -> Observation<P::Handle> {
        let handle = self
            .current
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|entry| Arc::clone(&entry.handle));
        let connected = handle
            .as_ref()
            .is_some_and(|current_handle| current_handle.is_connected());
        Observation {
            generation: self.generation.load(Ordering::SeqCst),
            handle,
            connected,
        }
    }

    pub async fn connect(&self, timeout: Option<Duration>) -> Result<(), ConnectError<P::Error>> {
        let candidate = self
            .current
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|entry| Arc::clone(&entry.handle))
            .ok_or(ConnectError::NotConnected)?;
        candidate
            .connect(timeout)
            .await
            .map_err(|source| ConnectError::Connect { source })?;
        self.instrument(&candidate);
        Ok(())
    }

    pub(crate) fn instrument(&self, candidate: &Arc<P::Handle>) -> Option<Arc<DrainTracker>> {
        let mut current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = current
            .as_mut()
            .filter(|entry| Arc::ptr_eq(&entry.handle, candidate))
        {
            if let Some(existing) = &entry.tracker {
                return Some(Arc::clone(existing));
            }
            if !candidate.is_connected() {
                return None;
            }
            let tracker = Arc::new(DrainTracker::new());
            entry.tracker = Some(Arc::clone(&tracker));
            return Some(tracker);
        }
        drop(current);
        if !candidate.is_connected() {
            return None;
        }
        Some(Arc::new(DrainTracker::new()))
    }

    pub async fn shutdown(&self) {
        let mut retirements = mem::take(
            &mut *self
                .retirements
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        retirements.abort_all();
        while retirements.join_next().await.is_some() {}
        let mut refreshes = mem::take(
            &mut *self
                .refreshes
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        refreshes.abort_all();
        while refreshes.join_next().await.is_some() {}
    }
}

// A joined task leaves its set only when the set is polled, so every spawn site reaps first.
pub(super) fn drop_finished(tasks: &mut JoinSet<()>) {
    while tasks.try_join_next().is_some() {}
}
