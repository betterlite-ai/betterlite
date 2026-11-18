use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::Notify;

// DailySpendUpdateQueue.interrupted_commits: the settlements a daily queue still owes, counted so a settle returns as soon as the last one ends.
pub(super) struct CommitProgress {
    pub(super) pending: AtomicUsize,
    pub(super) idle: Notify,
}

// The done callback of one settlement: it leaves the count the moment the settlement ends, is dropped or panics.
pub(super) struct SettleCompletion {
    pub(super) progress: Arc<CommitProgress>,
}

impl Drop for SettleCompletion {
    fn drop(&mut self) {
        // The matching increment happens before the task is spawned, so the count never goes below zero.
        let remaining = self.progress.pending.fetch_sub(1, Ordering::SeqCst);
        if remaining == 1 {
            self.progress.idle.notify_waiters();
        }
    }
}
