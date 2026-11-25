use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_db::budget_sweep::lease_guard::{LeaseGuard, LockReleaser};
use betterlite_db::queues::pod_lock::{LockCache, PodLockManager};
use tokio_util::task::TaskTracker;

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
