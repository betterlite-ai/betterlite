use std::convert::Infallible;
use std::error::Error;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};

use secrecy::SecretString;

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
    let previous = publish(generation, Arc::clone(&candidate), Some(tracker));
    generation
        .generation
        .store(old_generation.saturating_add(1), Ordering::SeqCst);
    invoke_replaced(generation.on_replaced.as_ref())
        .map_err(|source| ReplacementError::Callback { source })?;
    schedule_retirement(generation, previous, old_generation);
    Ok(())
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
    if let Some(previous) = clone_current(&generation.current) {
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
    drop(publish(generation, Arc::clone(&replacement), None));
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
    invoke_replaced(generation.on_replaced.as_ref())
        .map_err(|source| ReplacementError::Callback { source })?;
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
