use async_trait::async_trait;

use betterlite_db::queues::pod_lock::{LockCache, PodLockManager};

// The part of the pod lock manager an export job uses; the composition root passes the manager in.
#[async_trait]
pub trait UsageExportLease: Send + Sync {
    fn redis_available(&self) -> bool;

    async fn acquire_lock(&self, cronjob_id: &str) -> Option<bool>;

    async fn release_lock(&self, cronjob_id: &str);
}

#[async_trait]
impl<Cache: LockCache> UsageExportLease for PodLockManager<Cache> {
    fn redis_available(&self) -> bool {
        self.cache.is_some()
    }

    async fn acquire_lock(&self, cronjob_id: &str) -> Option<bool> {
        Self::acquire_lock(self, cronjob_id, None, true).await
    }

    async fn release_lock(&self, cronjob_id: &str) {
        Self::release_lock(self, cronjob_id).await;
    }
}
