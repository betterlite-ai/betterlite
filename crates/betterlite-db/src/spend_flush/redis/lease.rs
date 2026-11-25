use futures_util::future::BoxFuture;

use crate::queues::pod_lock::{LockCache, PodLockManager};

// A pod's lease over one spend cron job. The source calls PodLockManager.acquire_lock with no explicit TTL and allow_reentrant=True, and a manager without a Redis client answers None.
pub trait SpendPodLease: Send + Sync {
    fn acquire<'lease>(&'lease self, cronjob_id: &'lease str) -> BoxFuture<'lease, Option<bool>>;

    fn release<'lease>(&'lease self, cronjob_id: &'lease str) -> BoxFuture<'lease, ()>;
}

impl<Cache> SpendPodLease for PodLockManager<Cache>
where
    Cache: LockCache,
{
    fn acquire<'lease>(&'lease self, cronjob_id: &'lease str) -> BoxFuture<'lease, Option<bool>> {
        Box::pin(self.acquire_lock(cronjob_id, None, true))
    }

    fn release<'lease>(&'lease self, cronjob_id: &'lease str) -> BoxFuture<'lease, ()> {
        Box::pin(self.release_lock(cronjob_id))
    }
}
