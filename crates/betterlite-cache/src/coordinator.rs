use std::error::Error;
use std::future::Future;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;
use thiserror::Error;
use tokio::sync::watch;
use tokio::time::timeout;

use crate::backend::Ttl;

#[path = "coordinator/tests.rs"]
mod cache_coordination;

#[async_trait]
pub trait AsyncCacheProtocol: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    async fn async_get_cache(&self, key: &str) -> Result<Option<CacheValue>, Self::Error>;

    async fn async_set_cache(
        &self,
        key: &str,
        value: CacheValue,
        ttl: Ttl,
    ) -> Result<(), Self::Error>;
}

#[derive(Debug, Error)]
pub enum CoordinatorError<CacheError, LoadError> {
    #[error("the cache operation for {key} failed")]
    Cache {
        key: String,
        #[source]
        source: CacheError,
    },
    #[error("loading the value for {key} failed")]
    Load {
        key: String,
        #[source]
        source: LoadError,
    },
    #[error("no load signal for {key} arrived within {timeout:?}")]
    WaitTimeout { key: String, timeout: Duration },
    #[error("the load signal for {key} was dropped before it was set")]
    SignalClosed { key: String },
}

pub struct EventDrivenCacheCoordinator {
    state: Mutex<LoadState>,
    log_prefix: String,
    wait_timeout: Duration,
}

struct LoadState {
    load_in_progress: bool,
    signal: Option<watch::Sender<bool>>,
}

impl EventDrivenCacheCoordinator {
    // wait_timeout bounds the waiters, which LiteLLM leaves waiting for as long as the loader runs
    #[must_use]
    pub const fn new(log_prefix: String, wait_timeout: Duration) -> Self {
        Self {
            state: Mutex::new(LoadState {
                load_in_progress: false,
                signal: None,
            }),
            log_prefix,
            wait_timeout,
        }
    }

    pub async fn get_or_load<Cache, Load, LoadError>(
        &self,
        cache_key: &str,
        cache: &Cache,
        load_fn: Load,
    ) -> Result<Option<CacheValue>, CoordinatorError<Cache::Error, LoadError>>
    where
        Cache: AsyncCacheProtocol,
        Load: AsyncFnOnce() -> Result<CacheValue, LoadError>,
        LoadError: Error,
    {
        if let Some(value) = cache_read(cache, cache_key).await? {
            self.log_step("Cache hit");
            return Ok(Some(value));
        }
        self.log_step("Cache miss");
        let Some(signal) = self.claim_role() else {
            let _signal = LoadSignal { coordinator: self };
            let value = Self::load_and_cache(self, cache, cache_key, load_fn()).await?;
            return Ok(value);
        };
        self.wait_for_signal_and_get(signal, cache_key, cache).await
    }

    fn claim_role(&self) -> Option<watch::Receiver<bool>> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.load_in_progress
            && let Some(signal) = &state.signal
        {
            let waiter = signal.subscribe();
            drop(state);
            self.log_step("Load in flight, waiting for signal");
            return Some(waiter);
        }
        state.load_in_progress = true;
        let (sender, _waiter) = watch::channel(false);
        state.signal = Some(sender);
        drop(state);
        self.log_step("Starting load (will signal others when done)");
        None
    }

    fn signal_done(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.load_in_progress = false;
        let pending = state.signal.take();
        drop(state);
        if let Some(signal) = pending {
            self.log_step("Signaling all waiting requests");
            signal.send_replace(true);
        }
    }

    async fn wait_for_signal_and_get<Cache, LoadError>(
        &self,
        mut signal: watch::Receiver<bool>,
        cache_key: &str,
        cache: &Cache,
    ) -> Result<Option<CacheValue>, CoordinatorError<Cache::Error, LoadError>>
    where
        Cache: AsyncCacheProtocol,
        LoadError: Error,
    {
        match timeout(self.wait_timeout, signal.changed()).await {
            Ok(Ok(())) => {}
            Ok(Err(_closed)) => {
                return Err(CoordinatorError::SignalClosed {
                    key: cache_key.to_owned(),
                });
            }
            Err(_elapsed) => {
                return Err(CoordinatorError::WaitTimeout {
                    key: cache_key.to_owned(),
                    timeout: self.wait_timeout,
                });
            }
        }
        self.log_step("Signal received, reading from cache");
        let value = cache_read(cache, cache_key).await?;
        if value.is_some() {
            self.log_step("Cache filled by other request");
        } else {
            self.log_step("Signal received but cache still empty");
        }
        Ok(value)
    }

    // the loader rechecks the cache, loads once and signals the waiters through its guard
    async fn load_and_cache<Cache, Load, LoadError>(
        owner: &Self,
        cache: &Cache,
        cache_key: &str,
        load: Load,
    ) -> Result<Option<CacheValue>, CoordinatorError<Cache::Error, LoadError>>
    where
        Cache: AsyncCacheProtocol,
        Load: Future<Output = Result<CacheValue, LoadError>>,
        LoadError: Error,
    {
        let rechecked = cache_read(cache, cache_key).await?;
        if let Some(value) = rechecked {
            owner.log_step("Cache filled while acquiring lock");
            return Ok(Some(value));
        }
        owner.log_step("Running load");
        let value = load.await.map_err(|source| CoordinatorError::Load {
            key: cache_key.to_owned(),
            source,
        })?;
        cache
            .async_set_cache(cache_key, value.clone(), None)
            .await
            .map_err(|source| CoordinatorError::Cache {
                key: cache_key.to_owned(),
                source,
            })?;
        owner.log_step("Result cached");
        if matches!(value, CacheValue::Null) {
            return Ok(None);
        }
        Ok(Some(value))
    }

    fn log_step(&self, message: &str) {
        if !self.log_prefix.is_empty() {
            tracing::debug!("{} {message}", self.log_prefix);
        }
    }
}

// the Rust finally block: dropping the loader's guard signals waiters on error and on cancellation too
struct LoadSignal<'coordinator> {
    coordinator: &'coordinator EventDrivenCacheCoordinator,
}

impl Drop for LoadSignal<'_> {
    fn drop(&mut self) {
        self.coordinator.signal_done();
    }
}

async fn cache_read<Cache, LoadError>(
    cache: &Cache,
    cache_key: &str,
) -> Result<Option<CacheValue>, CoordinatorError<Cache::Error, LoadError>>
where
    Cache: AsyncCacheProtocol,
    LoadError: Error,
{
    let value = cache.async_get_cache(cache_key).await;
    let stored = value.map_err(|source| CoordinatorError::Cache {
        key: cache_key.to_owned(),
        source,
    })?;
    // LiteLLM tests `value is not None`, and a cached None reads back as a stored null here
    Ok(stored.filter(|entry| !matches!(entry, CacheValue::Null)))
}
