use std::collections::BTreeMap;
use std::mem;
use std::sync::{Mutex, PoisonError};

use super::rows::{FunnelCounters, FunnelStage, ShadowTables};

// the pod-local queue the success hook records into; the spend-update job drains it
#[derive(Debug, Default)]
pub struct FunnelQueue {
    pending: Mutex<BTreeMap<String, FunnelCounters>>,
}

impl FunnelQueue {
    // synchronous on purpose: a read-modify-write here cannot interleave with a flush snapshot
    pub fn record(&self, job_id: &str, stage: FunnelStage) {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(job_id.to_owned())
            .or_default()
            .record(stage);
    }

    #[must_use]
    pub fn pending_events(&self) -> i64 {
        let pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let mut total = 0_i64;
        for counters in pending.values() {
            total = total.saturating_add(counters.total());
        }
        drop(pending);
        total
    }

    // snapshot and clear first, then upsert: events recorded during the flush belong to the next batch
    pub async fn flush<Database>(&self, database: &Database)
    where
        Database: ShadowTables,
    {
        let batch = {
            let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            if pending.is_empty() {
                return;
            }
            mem::take(&mut *pending)
        };
        for (job_id, counters) in batch {
            if let Err(error) = database.upsert_funnel(&job_id, counters).await {
                tracing::error!(
                    "Spend tracking - shadow eval funnel flush failed for job {job_id}, {counters:?} dropped: {error}"
                );
            }
        }
    }
}
