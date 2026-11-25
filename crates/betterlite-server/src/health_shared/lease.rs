use std::sync::Arc;

use betterlite_db::budget_sweep::lease_guard::LockReleaser;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::task::TaskTracker;

// One held job's lease: a tracked task releases it on failure or cancellation, and a completed save keeps it.
pub struct WindowLease {
    release_now: Option<Arc<Notify>>,
    release_task: Option<JoinHandle<()>>,
}

impl WindowLease {
    #[must_use]
    pub fn start<L>(holder: Arc<L>, job: &'static str, tasks: &TaskTracker) -> Self
    where
        L: LockReleaser + ?Sized + 'static,
    {
        let release_now = Arc::new(Notify::new());
        let wake = Arc::clone(&release_now);
        let release_task = tasks.spawn(async move {
            wake.notified().await;
            holder.release(job).await;
        });
        Self {
            release_now: Some(release_now),
            release_task: Some(release_task),
        }
    }

    pub async fn release(mut self) {
        if let Some(wake) = self.release_now.take() {
            wake.notify_one();
        }
        if let Some(release_task) = self.release_task.take() {
            let _released = release_task.await;
        }
    }

    // A completed save is this window's marker: the lock expires with the interval instead of being deleted.
    pub fn keep(mut self) {
        self.release_now = None;
        if let Some(release_task) = self.release_task.take() {
            release_task.abort();
        }
    }
}

impl Drop for WindowLease {
    fn drop(&mut self) {
        // A cancelled save never reaches release(), so the tracked task finishes the unlock after the drop.
        if let Some(wake) = self.release_now.take() {
            wake.notify_one();
        }
    }
}
