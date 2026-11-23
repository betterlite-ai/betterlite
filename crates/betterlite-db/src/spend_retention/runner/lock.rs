use std::future::Future;

use crate::queues::pod_lock::{LockCache, PodLockManager};

use super::error::LockReleaseError;

// the slice of B1690's pod lock one run uses; a manager without a cache means "run without a lock", not "skip"
pub trait CleanupLock: Send + Sync {
    fn redis_backed(&self) -> bool;

    fn acquire(&self, cronjob_id: &str) -> impl Future<Output = Option<bool>> + Send;

    fn release(
        &self,
        cronjob_id: &str,
    ) -> impl Future<Output = Result<(), LockReleaseError>> + Send;
}

impl<Cache: LockCache> CleanupLock for PodLockManager<Cache> {
    fn redis_backed(&self) -> bool {
        self.cache.is_some()
    }

    async fn acquire(&self, cronjob_id: &str) -> Option<bool> {
        Self::acquire_lock(self, cronjob_id, None, true).await
    }

    // release_lock swallows its own cache errors like the source's, so a failing release cannot hide a run's result
    async fn release(&self, cronjob_id: &str) -> Result<(), LockReleaseError> {
        Self::release_lock(self, cronjob_id).await;
        Ok(())
    }
}
