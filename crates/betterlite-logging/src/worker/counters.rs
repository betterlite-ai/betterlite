use std::sync::atomic::{AtomicUsize, Ordering};

use super::{ShutdownReport, WorkerStats};

#[derive(Debug, Default)]
pub(super) struct Counters {
    pub(super) accepted: AtomicUsize,
    pub(super) rejected_full: AtomicUsize,
    pub(super) rejected_closed: AtomicUsize,
    pub(super) dequeued: AtomicUsize,
    pub(super) started: AtomicUsize,
    pub(super) completed: AtomicUsize,
    pub(super) failed: AtomicUsize,
    pub(super) timed_out: AtomicUsize,
    pub(super) cancelled: AtomicUsize,
}

impl Counters {
    pub(super) fn settled(&self) -> usize {
        self.completed
            .load(Ordering::SeqCst)
            .saturating_add(self.failed.load(Ordering::SeqCst))
            .saturating_add(self.timed_out.load(Ordering::SeqCst))
            .saturating_add(self.cancelled.load(Ordering::SeqCst))
    }
}

pub(super) const fn shutdown_report(stats: WorkerStats, drained: bool) -> ShutdownReport {
    let settled = stats
        .completed
        .saturating_add(stats.failed)
        .saturating_add(stats.timed_out)
        .saturating_add(stats.cancelled);
    ShutdownReport {
        accepted: stats.accepted,
        completed: stats.completed,
        failed: stats.failed,
        timed_out: stats.timed_out,
        cancelled: stats
            .cancelled
            .saturating_add(stats.started.saturating_sub(settled)),
        dropped: stats.accepted.saturating_sub(stats.started),
        drained,
    }
}
