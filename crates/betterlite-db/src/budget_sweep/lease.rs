use std::sync::Arc;

use crate::queues::pod_lock::owner::{decode_pod_owner, lease_key};
use crate::queues::pod_lock::{LockCache, PodLockManager};

pub const RESET_JOB_NAME: &str = "reset_budget_job";
// Fixed at 900 seconds in the source, however DEFAULT_CRON_JOB_LOCK_TTL_SECONDS is configured.
pub const RESET_LEASE_TTL_SECONDS: i64 = 900;

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
        .acquire_lock(RESET_JOB_NAME, Some(RESET_LEASE_TTL_SECONDS), true)
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
    let outcome = cache.get_owner(&lease_key(RESET_JOB_NAME)).await;
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
