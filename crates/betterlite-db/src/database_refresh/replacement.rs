use std::convert::Infallible;
use std::error::Error;
use std::mem;
use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use secrecy::SecretString;
use tokio::time::timeout;

use super::access::RefreshTrigger;
use super::generation::{ActivePool, PoolGeneration};
use super::handle::{PoolHandle, PoolProvider, PoolTransport};
use super::tracker::DrainTracker;

pub const RETIREMENT_DRAIN_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, thiserror::Error)]
pub enum ReplacementError<PoolError, CallbackError = Infallible>
where
    PoolError: Error + Send + Sync + 'static,
    CallbackError: Error + Send + Sync + 'static,
{
    #[error("no database URL is set for the replacement pool")]
    MissingUrl,
    #[error("the replacement database pool could not be built")]
    Build {
        #[source]
        source: PoolError,
    },
    #[error("the replacement database pool could not connect")]
    Connect {
        #[source]
        source: PoolError,
    },
    #[error("the connected replacement pool could not be instrumented")]
    Untracked,
    #[error("the replacement callback failed")]
    Callback {
        #[source]
        source: CallbackError,
    },
}

struct CandidateGuard<H>
where
    H: PoolHandle,
{
    handle: Option<Arc<H>>,
}

impl<H> Drop for CandidateGuard<H>
where
    H: PoolHandle,
{
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.close();
        }
    }
}

pub async fn recreate<P, T, C>(
    generation: &PoolGeneration<P, T, C>,
    new_db_url: &SecretString,
    transport: Option<&PoolTransport>,
    expected_generation: Option<u64>,
) -> Result<bool, ReplacementError<P::Error, C>>
where
    P: PoolProvider + 'static,
    T: RefreshTrigger + 'static,
    C: Error + Send + Sync + 'static,
    P::Handle: 'static,
{
    let _guard = generation.replacement_lock.lock().await;
    recreate_locked(generation, new_db_url, transport, expected_generation).await
}

pub async fn replace_locked<P, T, C>(
    generation: &PoolGeneration<P, T, C>,
    new_db_url: &SecretString,
    transport: Option<&PoolTransport>,
) -> Result<(), ReplacementError<P::Error, C>>
where
    P: PoolProvider + 'static,
    T: RefreshTrigger + 'static,
    C: Error + Send + Sync + 'static,
    P::Handle: 'static,
{
    let Some(url) = replacement_url(generation, new_db_url) else {
        return Err(ReplacementError::MissingUrl);
    };
    let built = generation
        .provider
        .build(&url, transport)
        .map_err(|source| ReplacementError::Build { source })?;
    let candidate = Arc::new(built);
    let mut pending = CandidateGuard {
        handle: Some(Arc::clone(&candidate)),
    };
    candidate
        .connect(None)
        .await
        .map_err(|source| ReplacementError::Connect { source })?;
    let Some(tracker) = generation.instrument(&candidate) else {
        return Err(ReplacementError::Untracked);
    };
    pending.handle.take();
    let old_generation = generation.generation.load(Ordering::SeqCst);
    let previous = publish_tracked(generation, Arc::clone(&candidate), tracker);
    generation
        .generation
        .store(old_generation.saturating_add(1), Ordering::SeqCst);
    invoke_replaced(generation).map_err(|source| ReplacementError::Callback { source })?;
    schedule_retirement(generation, previous, old_generation);
    Ok(())
}

pub async fn wait_for_pool_replacement<P, T, C>(
    generation: &PoolGeneration<P, T, C>,
    wait: Duration,
) where
    P: PoolProvider,
    T: RefreshTrigger,
    C: Error + Send + Sync + 'static,
{
    if wait.is_zero() {
        return;
    }
    if generation.replacement_lock.try_lock().is_ok() {
        return;
    }
    let Ok(_settled) = timeout(wait, generation.replacement_lock.lock()).await else {
        return;
    };
}

#[must_use]
pub fn retirement_count<P, T, C>(generation: &PoolGeneration<P, T, C>) -> usize
where
    P: PoolProvider,
    T: RefreshTrigger,
    C: Error + Send + Sync + 'static,
{
    let mut retirements = generation
        .retirements
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    while retirements.try_join_next().is_some() {}
    retirements.len()
}

pub async fn shutdown<P, T, C>(generation: &PoolGeneration<P, T, C>)
where
    P: PoolProvider,
    T: RefreshTrigger,
    C: Error + Send + Sync + 'static,
{
    let mut retirements = mem::take(
        &mut *generation
            .retirements
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    );
    retirements.abort_all();
    while retirements.join_next().await.is_some() {}
    let mut refreshes = mem::take(
        &mut *generation
            .refreshes
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    );
    refreshes.abort_all();
    while refreshes.join_next().await.is_some() {}
}

async fn recreate_locked<P, T, C>(
    generation: &PoolGeneration<P, T, C>,
    new_db_url: &SecretString,
    transport: Option<&PoolTransport>,
    expected_generation: Option<u64>,
) -> Result<bool, ReplacementError<P::Error, C>>
where
    P: PoolProvider + 'static,
    T: RefreshTrigger + 'static,
    C: Error + Send + Sync + 'static,
    P::Handle: 'static,
{
    let current_generation = generation.generation.load(Ordering::SeqCst);
    if let Some(expected) = expected_generation
        && expected != current_generation
    {
        let prefix = generation.options.log_prefix.as_str();
        tracing::info!(
            "{prefix}Skipping database pool replacement: generation already replaced (current {current_generation} != expected {expected})."
        );
        return Ok(false);
    }
    if let Some(previous) = clone_current(generation) {
        previous.handle.close();
    }
    let Some(url) = replacement_url(generation, new_db_url) else {
        return Err(ReplacementError::MissingUrl);
    };
    let built = generation
        .provider
        .build(&url, transport)
        .map_err(|source| ReplacementError::Build { source })?;
    let replacement = Arc::new(built);
    publish_untracked(generation, Arc::clone(&replacement));
    replacement
        .connect(None)
        .await
        .map_err(|source| ReplacementError::Connect { source })?;
    let Some(_tracker) = generation.instrument(&replacement) else {
        return Err(ReplacementError::Untracked);
    };
    generation
        .generation
        .store(current_generation.saturating_add(1), Ordering::SeqCst);
    invoke_replaced(generation).map_err(|source| ReplacementError::Callback { source })?;
    Ok(true)
}

fn replacement_url<P, T, C>(
    generation: &PoolGeneration<P, T, C>,
    new_db_url: &SecretString,
) -> Option<SecretString>
where
    P: PoolProvider,
    T: RefreshTrigger,
    C: Error + Send + Sync + 'static,
{
    if generation.options.recreate_uses_datasource {
        return Some(new_db_url.clone());
    }
    generation.options.url.snapshot()
}

fn clone_current<P, T, C>(generation: &PoolGeneration<P, T, C>) -> Option<ActivePool<P::Handle>>
where
    P: PoolProvider,
    T: RefreshTrigger,
    C: Error + Send + Sync + 'static,
{
    match generation.current.lock() {
        Ok(current) => current.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn publish_untracked<P, T, C>(generation: &PoolGeneration<P, T, C>, handle: Arc<P::Handle>)
where
    P: PoolProvider,
    T: RefreshTrigger,
    C: Error + Send + Sync + 'static,
{
    generation
        .current
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .replace(ActivePool {
            handle,
            tracker: None,
        });
}

fn publish_tracked<P, T, C>(
    generation: &PoolGeneration<P, T, C>,
    handle: Arc<P::Handle>,
    tracker: Arc<DrainTracker>,
) -> Option<ActivePool<P::Handle>>
where
    P: PoolProvider,
    T: RefreshTrigger,
    C: Error + Send + Sync + 'static,
{
    generation
        .current
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .replace(ActivePool {
            handle,
            tracker: Some(tracker),
        })
}

fn invoke_replaced<P, T, C>(generation: &PoolGeneration<P, T, C>) -> Result<(), C>
where
    P: PoolProvider,
    T: RefreshTrigger,
    C: Error + Send + Sync + 'static,
{
    let Some(callback) = &generation.on_replaced else {
        return Ok(());
    };
    callback()
}

fn schedule_retirement<P, T, C>(
    generation: &PoolGeneration<P, T, C>,
    retired_pool: Option<ActivePool<P::Handle>>,
    generation_number: u64,
) where
    P: PoolProvider + 'static,
    T: RefreshTrigger + 'static,
    C: Error + Send + Sync + 'static,
    P::Handle: 'static,
{
    let Some(retired) = retired_pool else {
        return;
    };
    let handle = retired.handle;
    let old_tracker = retired.tracker;
    let prefix = generation.options.log_prefix.clone();
    let task = async move {
        if let Some(active_tracker) = old_tracker
            && timeout(
                RETIREMENT_DRAIN_TIMEOUT,
                active_tracker.wait_until_drained(),
            )
            .await
            .is_err()
        {
            tracing::warn!(
                "{prefix}Replaced database pool generation {generation_number} did not drain within {}s; closing it with work still in flight.",
                RETIREMENT_DRAIN_TIMEOUT.as_secs()
            );
        }
        handle.close();
    };
    generation
        .retirements
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .spawn(task);
}
