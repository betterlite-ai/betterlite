use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_db::budget_sweep::lease_guard::{LeaseGuard, LockReleaser};
use betterlite_db::queues::pod_lock::{LockCache, PodLockManager};
use tokio_util::task::TaskTracker;

use super::cache::SharedHealthCache;
use super::cached::cached_owner;

// A completed save keeps the window's lock, so the lease's holder skips its delete.
struct WindowHolder<L: LockCache> {
    manager: Arc<PodLockManager<L>>,
    kept: Arc<AtomicBool>,
}

impl<L: LockCache> LockReleaser for WindowHolder<L> {
    async fn release(&self, cronjob_id: &str) {
        if !self.kept.load(Ordering::Acquire) {
            self.manager.release_lock(cronjob_id).await;
        }
    }
}

// One held job's lease: the tracked release task unlocks on failure or cancellation, and a completed save keeps the lock.
pub struct WindowLease {
    kept: Arc<AtomicBool>,
    guard: Option<LeaseGuard>,
}

impl WindowLease {
    #[must_use]
    pub fn start<L>(manager: Arc<PodLockManager<L>>, job: &'static str, tasks: &TaskTracker) -> Self
    where
        L: LockCache + 'static,
    {
        let kept = Arc::new(AtomicBool::new(false));
        let holder = Arc::new(WindowHolder {
            manager,
            kept: Arc::clone(&kept),
        });
        Self {
            kept,
            guard: Some(LeaseGuard::start(holder, job, tasks)),
        }
    }

    pub async fn release(mut self) {
        if let Some(guard) = self.guard.take() {
            guard.release().await;
        }
    }

    pub fn keep(mut self) {
        self.kept.store(true, Ordering::Release);
        self.guard = None;
    }
}

// The shared health check lock's holder, as the lease's releaser: a check future dropped mid-probe still unlocks, as the source's finally does.
struct LockOwner {
    redis: Arc<dyn SharedHealthCache>,
    pod_id: String,
}

impl LockReleaser for LockOwner {
    async fn release(&self, key: &str) {
        release_lock(self.redis.as_ref(), &self.pod_id, key).await;
    }
}

// The shared lock's tracked release: the check holds this lease from the moment it acquires until it answers.
pub(crate) fn shared_lock_lease(
    redis: &Arc<dyn SharedHealthCache>,
    pod_id: &str,
    key: &'static str,
    tasks: &TaskTracker,
) -> LeaseGuard {
    let holder = Arc::new(LockOwner {
        redis: Arc::clone(redis),
        pod_id: pod_id.to_owned(),
    });
    LeaseGuard::start(holder, key, tasks)
}

// A read-then-delete, never an atomic compare-delete: a lock another pod reacquired in between stays.
pub(crate) async fn release_lock(redis: &dyn SharedHealthCache, pod_id: &str, key: &str) {
    let owner = match redis.get(key).await {
        Ok(owner) => owner,
        Err(error) => {
            tracing::error!("Error releasing health check lock: {error}");
            return;
        }
    };
    let ours = owner
        .and_then(cached_owner)
        .is_some_and(|stored| stored == pod_id);
    if !ours {
        return;
    }
    match redis.delete(key).await {
        Ok(_deleted) => tracing::info!(pod = %pod_id, "pod released the health check lock"),
        Err(error) => tracing::error!("Error releasing health check lock: {error}"),
    }
}
