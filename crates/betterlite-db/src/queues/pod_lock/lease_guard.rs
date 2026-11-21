use std::future::Future;
use std::sync::Arc;

use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::task::TaskTracker;

use super::{LockCache, PodLockManager};

// The release side of a lease holder: the pod lock manager here, and the server's scheduled export ports built on it.
pub trait LockReleaser: Send + Sync {
    fn release(&self, cronjob_id: &str) -> impl Future<Output = ()> + Send;
}

impl<Cache: LockCache> LockReleaser for PodLockManager<Cache> {
    async fn release(&self, cronjob_id: &str) {
        self.release_lock(cronjob_id).await;
    }
}

// The lease of one held job: the tracked task releases it once, whether the job finishes or its future is dropped.
pub struct LeaseGuard {
    release_now: Arc<Notify>,
    release_task: Option<JoinHandle<()>>,
}

impl LeaseGuard {
    #[must_use]
    pub fn start<L>(holder: Arc<L>, cronjob_id: &'static str, tasks: &TaskTracker) -> Self
    where
        L: LockReleaser + 'static,
    {
        let release_now = Arc::new(Notify::new());
        let wake = Arc::clone(&release_now);
        let release_task = tasks.spawn(async move {
            wake.notified().await;
            holder.release(cronjob_id).await;
        });
        Self {
            release_now,
            release_task: Some(release_task),
        }
    }

    pub async fn release(mut self) {
        self.release_now.notify_one();
        if let Some(release_task) = self.release_task.take() {
            let _released = release_task.await;
        }
    }
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        // A cancelled job never reaches release(), so the tracked task finishes the unlock after the drop.
        self.release_now.notify_one();
    }
}
