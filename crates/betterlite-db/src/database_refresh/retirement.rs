use std::error::Error;
use std::mem;
use std::sync::PoisonError;
use std::time::Duration;

use tokio::time::timeout;

use super::access::RefreshTrigger;
use super::generation::{ActivePool, PoolGeneration};
use super::handle::{PoolHandle as _, PoolProvider};

pub const RETIREMENT_DRAIN_TIMEOUT: Duration = Duration::from_secs(90);

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

pub(super) fn schedule_retirement<P, T, C>(
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
