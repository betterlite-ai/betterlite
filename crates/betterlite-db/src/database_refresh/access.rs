use std::error::Error;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, PoisonError};
use std::thread;

use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::{Builder, Handle};

use super::generation::PoolGeneration;
use super::handle::{OperationHandle, PoolProvider};
use super::replacement::ReplacementError;

pub type RefreshFuture<'borrowed, E> =
    Pin<Box<dyn Future<Output = Result<(), E>> + Send + 'borrowed>>;

// The higher lifecycle layer mints and inspects tokens; this pool service only triggers it.
pub trait RefreshTrigger: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn token_expired(&self) -> bool;

    fn refresh(&self) -> RefreshFuture<'_, Self::Error>;

    fn minted_url(&self) -> Result<Option<SecretString>, Self::Error>;
}

#[derive(Debug, thiserror::Error)]
pub enum AccessError {
    #[error("no database pool handle is selected")]
    NotConnected,
}

#[derive(Debug, thiserror::Error)]
pub enum BlockingAccessError<PortError, PoolError, CallbackError>
where
    PortError: Error + Send + Sync + 'static,
    PoolError: Error + Send + Sync + 'static,
    CallbackError: Error + Send + Sync + 'static,
{
    #[error("no database pool handle is selected")]
    NotConnected,
    #[error("the database token could not be minted")]
    Mint {
        #[source]
        source: PortError,
    },
    #[error("Failed to get {label}")]
    MissingToken { label: &'static str },
    #[error("the database pool replacement failed")]
    Replace {
        #[source]
        source: ReplacementError<PoolError, CallbackError>,
    },
    #[error("the synchronous refresh could not start its own runtime")]
    Runtime {
        #[source]
        source: io::Error,
    },
    #[error("the synchronous refresh thread ended unexpectedly")]
    ThreadPanicked,
}

pub fn operation<P, T, C>(
    generation: &PoolGeneration<P, T, C>,
) -> Result<OperationHandle<P::Handle>, AccessError>
where
    P: PoolProvider,
    T: RefreshTrigger + 'static,
    C: Error + Send + Sync + 'static,
{
    let handle = connected_operation(generation).ok_or(AccessError::NotConnected)?;
    if generation.options.token_auth.is_none() {
        return Ok(handle);
    }
    let Some(port) = &generation.refresh else {
        return Ok(handle);
    };
    if !port.token_expired() {
        return Ok(handle);
    }
    let prefix = generation.options.log_prefix.as_str();
    let label = generation.token_label();
    tracing::warn!(
        "{prefix}{label} expired during database access - proactive refresh may have failed. Scheduling async refresh; the current request may fail and be retried with the fresh token."
    );
    schedule_refresh(generation, Arc::clone(port));
    Ok(handle)
}

pub type BlockingResult<P, T, C> = Result<
    OperationHandle<<P as PoolProvider>::Handle>,
    BlockingAccessError<<T as RefreshTrigger>::Error, <P as PoolProvider>::Error, C>,
>;

pub fn operation_blocking<P, T, C>(
    generation: &Arc<PoolGeneration<P, T, C>>,
) -> BlockingResult<P, T, C>
where
    P: PoolProvider + 'static,
    T: RefreshTrigger + 'static,
    C: Error + Send + Sync + 'static,
    P::Handle: 'static,
{
    let handle = connected_operation(generation).ok_or(BlockingAccessError::NotConnected)?;
    if generation.options.token_auth.is_none() {
        return Ok(handle);
    }
    let Some(port) = &generation.refresh else {
        return Ok(handle);
    };
    if !port.token_expired() {
        return Ok(handle);
    }
    let prefix = generation.options.log_prefix.as_str();
    let label = generation.token_label();
    tracing::warn!(
        "{prefix}{label} expired during database access - proactive refresh may have failed. Triggering synchronous fallback refresh..."
    );
    let minted = match port.minted_url() {
        Ok(minted) => minted,
        Err(source) => return Err(BlockingAccessError::Mint { source }),
    };
    let Some(fresh_url) = minted.filter(|url| !url.expose_secret().is_empty()) else {
        return Err(BlockingAccessError::MissingToken { label });
    };
    recreate_on_thread(generation, &fresh_url)?;
    let refreshed = connected_operation(generation).ok_or(BlockingAccessError::NotConnected)?;
    tracing::info!("{prefix}Synchronous token refresh completed successfully");
    Ok(refreshed)
}

fn connected_operation<P, T, C>(
    generation: &PoolGeneration<P, T, C>,
) -> Option<OperationHandle<P::Handle>>
where
    P: PoolProvider,
    T: RefreshTrigger,
    C: Error + Send + Sync + 'static,
{
    let current = generation
        .current
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let entry = current.as_ref()?;
    let handle = Arc::clone(&entry.handle);
    let tracker = entry.tracker.as_ref().map(Arc::clone);
    drop(current);
    Some(OperationHandle { handle, tracker })
}

fn schedule_refresh<P, T, C>(generation: &PoolGeneration<P, T, C>, port: Arc<T>)
where
    P: PoolProvider,
    T: RefreshTrigger + 'static,
    C: Error + Send + Sync + 'static,
{
    if Handle::try_current().is_err() {
        tracing::error!("cannot schedule a database token refresh without a tokio runtime");
        return;
    }
    let prefix = generation.options.log_prefix.clone();
    let label = generation.token_label();
    let task = async move {
        if let Err(error) = port.refresh().await {
            tracing::error!("{prefix}Background {label} refresh failed: {error}");
        }
    };
    generation
        .refreshes
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .spawn(task);
}

fn recreate_on_thread<P, T, C>(
    generation: &Arc<PoolGeneration<P, T, C>>,
    url: &SecretString,
) -> Result<(), BlockingAccessError<T::Error, P::Error, C>>
where
    P: PoolProvider + 'static,
    T: RefreshTrigger + 'static,
    C: Error + Send + Sync + 'static,
    P::Handle: 'static,
{
    let shared = Arc::clone(generation);
    let fresh_url = url.clone();
    let worker = thread::Builder::new()
        .name("database-pool-refresh".to_owned())
        .spawn(move || {
            let runtime = Builder::new_current_thread().enable_all().build()?;
            Ok(runtime.block_on(super::replacement::recreate(
                &shared, &fresh_url, None, None,
            )))
        })
        .map_err(|source| BlockingAccessError::Runtime { source })?;
    let joined = worker
        .join()
        .map_err(|_panic| BlockingAccessError::ThreadPanicked)?;
    let replaced = joined.map_err(|source| BlockingAccessError::Runtime { source })?;
    match replaced {
        Ok(_replaced) => Ok(()),
        Err(source) => Err(BlockingAccessError::Replace { source }),
    }
}
