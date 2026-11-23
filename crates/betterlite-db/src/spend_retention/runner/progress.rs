use std::sync::atomic::{AtomicU64, Ordering};

use crate::spend_retention::batches::BatchProgress;

// _RunProgress: where one run got to, readable by the cancellation guard that outlives the batches
#[derive(Debug, Default)]
pub struct RunProgress {
    rows_deleted: AtomicU64,
    batches: AtomicU64,
}

impl RunProgress {
    #[must_use]
    pub fn counts(&self) -> (u64, u64) {
        let rows_deleted = self.rows_deleted.load(Ordering::Relaxed);
        let batches = self.batches.load(Ordering::Relaxed);
        (rows_deleted, batches)
    }
}

// _record_run_batch: the batch deleter counts into the run's progress through a shared handle
pub struct SharedProgress<'run>(pub &'run RunProgress);

impl BatchProgress for SharedProgress<'_> {
    fn record_batch(&mut self, rows_deleted: u64) {
        self.0
            .rows_deleted
            .fetch_add(rows_deleted, Ordering::Relaxed);
        self.0.batches.fetch_add(1, Ordering::Relaxed);
    }
}
