#![cfg(test)]

use std::sync::Arc;

use crate::finops::focus::export_schedule::{FOCUS_USAGE_DATA_JOB_NAME, FocusPodLock};
use crate::finops::focus::support::{self, SharedLockCache};

#[test]
fn the_port_reports_whether_a_cache_is_configured() {
    assert!(!support::lock_manager(None).has_cache());
    assert!(support::lock_manager(Some(Arc::new(SharedLockCache::default()))).has_cache());
}

#[tokio::test]
async fn the_lease_owner_reacquires_the_lease_while_a_rival_pod_is_denied_until_release() {
    let shared = SharedLockCache::default();
    let lease_owner = support::lock_manager(Some(Arc::new(shared.clone())));
    let rival_pod = support::lock_manager(Some(Arc::new(shared.clone())));
    assert_eq!(
        FocusPodLock::acquire_lock(lease_owner.as_ref(), FOCUS_USAGE_DATA_JOB_NAME).await,
        Some(true)
    );
    assert_eq!(
        FocusPodLock::acquire_lock(lease_owner.as_ref(), FOCUS_USAGE_DATA_JOB_NAME).await,
        Some(true)
    );
    assert_eq!(
        FocusPodLock::acquire_lock(rival_pod.as_ref(), FOCUS_USAGE_DATA_JOB_NAME).await,
        Some(false)
    );
    FocusPodLock::release_lock(lease_owner.as_ref(), FOCUS_USAGE_DATA_JOB_NAME).await;
    assert_eq!(
        FocusPodLock::acquire_lock(rival_pod.as_ref(), FOCUS_USAGE_DATA_JOB_NAME).await,
        Some(true)
    );
    assert_eq!(shared.ttls(), vec![60, 60, 60, 60]);
}
