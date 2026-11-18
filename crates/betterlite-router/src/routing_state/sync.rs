use std::sync::{Arc, Mutex, PoisonError};

use tokio::runtime::Handle;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::FLUSH_DEADLINE;
use super::delta::{self, DeltaStore, FlushGate, SyncOutcome, SyncState};

// the sink's saturated path: the owner's task set runs the flush, so a stalled store never blocks the callback
pub(super) fn schedule_flush<S>(state: &Arc<SyncState<S>>, flushes: &Mutex<JoinSet<()>>)
where
    S: DeltaStore + 'static,
{
    if Handle::try_current().is_err() {
        tracing::warn!(
            "the joined usage deltas need a running runtime to flush; they wait for the next sync"
        );
        return;
    }
    // the sink's path takes the gate without waiting, so a full queue never blocks the callback's thread
    let Ok(gate) = Arc::clone(&state.gate.held).try_lock_owned() else {
        return;
    };
    let owned = Arc::clone(state);
    let mut tasks = flushes.lock().unwrap_or_else(PoisonError::into_inner);
    while tasks.try_join_next().is_some() {}
    let _task = tasks.spawn(async move {
        let _guard = gate;
        // the task holds the gate already, so it pushes what joined it while it wrote
        if timeout(FLUSH_DEADLINE, delta::drain(&owned)).await.is_err() {
            delta::discard_joined(&owned);
            tracing::warn!("the scheduled routing flush passed its deadline");
        }
    });
}

// BaseRoutingStrategy._sync_in_memory_spend_with_redis: one bounded flush, then the merge of the store's totals
pub(super) async fn sync_state<S>(state: &SyncState<S>) -> SyncOutcome
where
    S: DeltaStore,
{
    if state.store.is_none() {
        return SyncOutcome::Empty;
    }
    let outcome = timeout(FLUSH_DEADLINE, async {
        let Some(_gate) = FlushGate::acquire(&state.gate, FLUSH_DEADLINE).await else {
            return SyncOutcome::Busy;
        };
        delta::drain(state).await
    })
    .await;
    match outcome {
        Ok(outcome) => outcome,
        Err(_elapsed) => {
            delta::discard_joined(state);
            tracing::warn!("the routing sync passed its deadline");
            SyncOutcome::Failed
        }
    }
}
