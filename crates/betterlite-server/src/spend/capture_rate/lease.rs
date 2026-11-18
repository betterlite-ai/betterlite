use betterlite_db::queues::pod_lock::owner::lease_key;
use betterlite_db::queues::pod_lock::{LockCache, PodLockManager};
use betterlite_models::capture_rate::{CHECK_JOB_ID, CHECK_LOCK_TTL_SECONDS};

// The source treats an absent cache as free to alert, and a failed acquisition defers to the lock's current owner.
pub(super) async fn claims_alert_window<C>(pod_lock_manager: Option<&PodLockManager<C>>) -> bool
where
    C: LockCache,
{
    let Some(pod_lock_manager) = pod_lock_manager else {
        return true;
    };
    if pod_lock_manager.cache.is_none() {
        return true;
    }
    let acquired = pod_lock_manager
        .acquire_lock(CHECK_JOB_ID, Some(CHECK_LOCK_TTL_SECONDS), true)
        .await;
    if acquired == Some(true) {
        return true;
    }
    !lock_is_held(pod_lock_manager).await
}

// An unreadable lock must not silence the alert, so a read failure counts as not held.
async fn lock_is_held<C>(pod_lock_manager: &PodLockManager<C>) -> bool
where
    C: LockCache,
{
    let Some(cache) = pod_lock_manager.cache.as_deref() else {
        return false;
    };
    match cache.get_owner(&lease_key(CHECK_JOB_ID)).await {
        Ok(owner) => owner.is_some_and(|stored| !stored.is_empty()),
        Err(error) => {
            tracing::warn!(
                error = %error,
                "Spend capture-rate check: could not read the lock"
            );
            false
        }
    }
}
