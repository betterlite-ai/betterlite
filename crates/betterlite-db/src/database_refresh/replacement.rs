use std::convert::Infallible;
use std::error::Error;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use secrecy::SecretString;
use tokio::time::timeout;

use super::access::RefreshTrigger;
use super::generation::{ActivePool, PoolGeneration};
use super::handle::{PoolHandle, PoolProvider, PoolTransport};
use super::retirement::schedule_retirement;
use super::tracker::DrainTracker;

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

struct Committed<H> {
    previous_generation: u64,
    previous: Option<ActivePool<H>>,
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
    P: PoolProvider,
    T: RefreshTrigger,
    C: Error + Send + Sync + 'static,
{
    let _guard = generation.replacement_lock.lock().await;
    if !generation_matches(generation, expected_generation) {
        return Ok(false);
    }
    let retired = clone_current(&generation.current);
    if let Some(previous) = retired {
        previous.handle.close();
    }
    let Some(url) = replacement_url(generation, new_db_url) else {
        return Err(ReplacementError::MissingUrl);
    };
    let replacement = build_candidate(generation.provider.as_ref(), &url, transport)?;
    drop(publish(generation, Arc::clone(&replacement), None));
    let tracker = connect_candidate(generation, &replacement).await?;
    let _committed = commit_replacement(generation, replacement, tracker)?;
    Ok(true)
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

// Caller holds replacement_lock; the candidate connects while the published pool keeps serving.
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
    let (candidate, tracker) = open_candidate(generation, new_db_url, transport).await?;
    let committed = commit_replacement(generation, candidate, tracker)?;
    schedule_retirement(
        generation,
        committed.previous,
        committed.previous_generation,
    );
    Ok(())
}

async fn open_candidate<P, T, C>(
    generation: &PoolGeneration<P, T, C>,
    new_db_url: &SecretString,
    transport: Option<&PoolTransport>,
) -> Result<(Arc<P::Handle>, Arc<DrainTracker>), ReplacementError<P::Error, C>>
where
    P: PoolProvider,
    T: RefreshTrigger,
    C: Error + Send + Sync + 'static,
{
    let Some(url) = replacement_url(generation, new_db_url) else {
        return Err(ReplacementError::MissingUrl);
    };
    let candidate = build_candidate(generation.provider.as_ref(), &url, transport)?;
    let mut pending = CandidateGuard {
        handle: Some(Arc::clone(&candidate)),
    };
    let tracker = connect_candidate(generation, &candidate).await?;
    pending.handle.take();
    Ok((candidate, tracker))
}

fn build_candidate<P, C>(
    provider: &P,
    url: &SecretString,
    transport: Option<&PoolTransport>,
) -> Result<Arc<P::Handle>, ReplacementError<P::Error, C>>
where
    P: PoolProvider,
    C: Error + Send + Sync + 'static,
{
    let built = provider
        .build(url, transport)
        .map_err(|source| ReplacementError::Build { source })?;
    Ok(Arc::new(built))
}

async fn connect_candidate<P, T, C>(
    generation: &PoolGeneration<P, T, C>,
    candidate: &Arc<P::Handle>,
) -> Result<Arc<DrainTracker>, ReplacementError<P::Error, C>>
where
    P: PoolProvider,
    T: RefreshTrigger,
    C: Error + Send + Sync + 'static,
{
    candidate
        .connect(None)
        .await
        .map_err(|source| ReplacementError::Connect { source })?;
    let Some(tracker) = generation.instrument(candidate) else {
        return Err(ReplacementError::Untracked);
    };
    Ok(tracker)
}

fn commit_replacement<P, T, C>(
    generation: &PoolGeneration<P, T, C>,
    handle: Arc<P::Handle>,
    tracker: Arc<DrainTracker>,
) -> Result<Committed<P::Handle>, ReplacementError<P::Error, C>>
where
    P: PoolProvider,
    T: RefreshTrigger,
    C: Error + Send + Sync + 'static,
{
    let previous_generation = generation.generation.load(Ordering::SeqCst);
    let previous = publish(generation, handle, Some(tracker));
    generation
        .generation
        .store(previous_generation.saturating_add(1), Ordering::SeqCst);
    invoke_replaced(generation.on_replaced.as_ref())
        .map_err(|source| ReplacementError::Callback { source })?;
    Ok(Committed {
        previous_generation,
        previous,
    })
}

fn generation_matches<P, T, C>(
    generation: &PoolGeneration<P, T, C>,
    expected_generation: Option<u64>,
) -> bool
where
    P: PoolProvider,
    T: RefreshTrigger,
    C: Error + Send + Sync + 'static,
{
    let Some(expected) = expected_generation else {
        return true;
    };
    let current = generation.generation.load(Ordering::SeqCst);
    if expected == current {
        return true;
    }
    let prefix = generation.options.log_prefix.as_str();
    tracing::info!(
        "{prefix}Skipping database pool replacement: generation already replaced (current {current} != expected {expected})."
    );
    false
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

fn clone_current<H>(current: &Mutex<Option<ActivePool<H>>>) -> Option<ActivePool<H>> {
    match current.lock() {
        Ok(slot) => slot.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn publish<P, T, C>(
    generation: &PoolGeneration<P, T, C>,
    handle: Arc<P::Handle>,
    tracker: Option<Arc<DrainTracker>>,
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
        .replace(ActivePool { handle, tracker })
}

fn invoke_replaced<C>(
    callback: Option<&Arc<dyn Fn() -> Result<(), C> + Send + Sync>>,
) -> Result<(), C> {
    let Some(settled) = callback else {
        return Ok(());
    };
    settled()
}
