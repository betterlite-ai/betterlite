use std::sync::Arc;

use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::task::TaskTracker;

use crate::queues::pod_lock::owner::{decode_pod_owner, lease_key};
use crate::queues::pod_lock::{LockCache, PodLockManager};

pub const RESET_BUDGET_JOB_NAME: &str = "reset_budget_job";
// Fixed at 900 seconds in the source, however DEFAULT_CRON_JOB_LOCK_TTL_SECONDS is configured.
pub const RESET_BUDGET_JOB_LOCK_TTL_SECONDS: i64 = 900;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lease {
    Leader,
    Unguarded,
    Follower,
}

pub(crate) async fn elect<Cache>(manager: Option<&Arc<PodLockManager<Cache>>>) -> Lease
where
    Cache: LockCache,
{
    let Some(lock_manager) = manager else {
        return Lease::Unguarded;
    };
    let Some(cache) = lock_manager.cache.as_deref() else {
        return Lease::Unguarded;
    };
    let acquired = lock_manager
        .acquire_lock(
            RESET_BUDGET_JOB_NAME,
            Some(RESET_BUDGET_JOB_LOCK_TTL_SECONDS),
            true,
        )
        .await;
    if acquired == Some(true) {
        return Lease::Leader;
    }
    if lease_is_held(cache).await {
        tracing::debug!("Reset budget job: another pod holds the reset lease, skipping this tick");
        return Lease::Follower;
    }
    // A failed acquire also covers an unreachable Redis, which must not read as contention and strand every due budget.
    tracing::warn!(
        "Reset budget job: could not take the reset lease and no other pod holds it, sweeping unguarded rather than skipping the tick"
    );
    Lease::Unguarded
}

// An unreadable lease reports as unheld so the caller sweeps rather than skipping; being wrong costs a duplicate sweep.
async fn lease_is_held<Cache>(cache: &Cache) -> bool
where
    Cache: LockCache,
{
    let outcome = cache.get_owner(&lease_key(RESET_BUDGET_JOB_NAME)).await;
    match outcome {
        Ok(Some(stored)) => match decode_pod_owner(&stored) {
            Ok(owner) => !owner.is_empty(),
            Err(error) => {
                tracing::warn!(error = %error, "Reset budget job: could not read the reset lease");
                false
            }
        },
        Ok(None) => false,
        Err(error) => {
            tracing::warn!(error = %error, "Reset budget job: could not read the reset lease");
            false
        }
    }
}

// The lease of one leader run: the tracked task releases it once, whether the run finishes or its future is dropped.
pub(crate) struct LeaseRelease {
    signal: Arc<Notify>,
    task: Option<JoinHandle<()>>,
}

impl LeaseRelease {
    #[must_use]
    pub(crate) fn start<Cache>(manager: Arc<PodLockManager<Cache>>, tasks: &TaskTracker) -> Self
    where
        Cache: LockCache + 'static,
    {
        let signal = Arc::new(Notify::new());
        let wake = Arc::clone(&signal);
        let task = tasks.spawn(async move {
            wake.notified().await;
            manager.release_lock(RESET_BUDGET_JOB_NAME).await;
        });
        Self {
            signal,
            task: Some(task),
        }
    }

    pub(crate) async fn release(mut self) {
        self.signal.notify_one();
        if let Some(task) = self.task.take() {
            let _released = task.await;
        }
    }
}

impl Drop for LeaseRelease {
    fn drop(&mut self) {
        // A cancelled run never reaches release(), so the tracked task finishes the unlock after the drop.
        self.signal.notify_one();
    }
}
