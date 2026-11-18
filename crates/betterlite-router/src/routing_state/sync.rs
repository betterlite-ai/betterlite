use tokio::runtime::{Handle, RuntimeFlavor};

use super::FLUSH_DEADLINE;
use super::counter::RouterCounters;
use super::delta::{self, Delta, DeltaStore, SyncOutcome};

impl<S: DeltaStore + 'static> RouterCounters<S> {
    // BaseRoutingStrategy._sync_in_memory_spend_with_redis
    pub async fn sync_with_redis(&self) -> bool {
        matches!(
            delta::sync_state(self.state.as_ref()).await,
            SyncOutcome::Pushed
        )
    }

    // the queue bound is the owner's batch size: a full queue flushes once in the caller's timeframe
    pub(super) async fn flush_or_drop(&self, delta: Delta) {
        let key = delta.key.as_ref().to_owned();
        let outcome =
            tokio::time::timeout(FLUSH_DEADLINE, delta::sync_state(self.state.as_ref())).await;
        let queued = match outcome {
            Ok(SyncOutcome::Pushed | SyncOutcome::Empty) => self.state.queue(delta),
            Ok(SyncOutcome::Failed | SyncOutcome::Busy) | Err(_) => false,
        };
        if queued {
            return;
        }
        tracing::warn!(
            key = key.as_str(),
            "the queue is full; the delta is dropped after the bounded flush"
        );
    }

    // B397's sink cannot await, so a saturated queue flushes the same way on the calling worker thread
    pub(super) fn flush_saturated(&self, delta: Delta) {
        let Some(handle) = worker_handle() else {
            tracing::warn!(
                key = delta.key.as_ref(),
                "the usage delta needs a multi-threaded runtime to flush; it is dropped"
            );
            return;
        };
        tokio::task::block_in_place(|| handle.block_on(self.flush_or_drop(delta)));
    }
}

// block_in_place only takes a worker's thread on the multi-threaded runtime, and the push needs one to run
fn worker_handle() -> Option<Handle> {
    match Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => Some(handle),
        Ok(_) | Err(_) => None,
    }
}
