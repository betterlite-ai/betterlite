use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::runtime::Handle;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::delta::{self, DeltaStore, FlushGate, SyncOutcome, SyncState};
use super::{FLUSH_DEADLINE, MAX_PENDING_DELTAS};

// callbacks claim a scheduled task independently of the gate held by a running flush
pub(super) fn schedule_flush<S>(state: &Arc<SyncState<S>>, flushes: &Mutex<JoinSet<()>>)
where
    S: DeltaStore + 'static,
{
    if Handle::try_current().is_err() {
        tracing::warn!(
            "the usage deltas need a running runtime to flush; they wait for the next sync"
        );
        return;
    }
    if state.flush_scheduled.swap(true, Ordering::AcqRel) {
        return;
    }
    let owned = Arc::clone(state);
    let mut tasks = flushes.lock().unwrap_or_else(PoisonError::into_inner);
    while tasks.try_join_next().is_some() {}
    let _task = tasks.spawn(async move {
        let _outcome = sync_state(&owned, true).await;
    });
}

// the deadline covers gate acquisition and every push; a callback can schedule a successor before the snapshot
pub(super) async fn sync_state<S>(state: &SyncState<S>, scheduled: bool) -> SyncOutcome
where
    S: DeltaStore,
{
    if state.store.is_none() {
        return SyncOutcome::Empty;
    }
    let mut waiting = true;
    let outcome = timeout(FLUSH_DEADLINE, async {
        let Some(_gate) = FlushGate::acquire(&state.gate, FLUSH_DEADLINE).await else {
            return SyncOutcome::Busy;
        };
        waiting = false;
        if scheduled {
            state.flush_scheduled.store(false, Ordering::Release);
        }
        push_batches(state).await
    })
    .await;
    if waiting {
        let mut pending = state.pending.lock().unwrap_or_else(PoisonError::into_inner);
        if scheduled {
            state.flush_scheduled.store(false, Ordering::Release);
        }
        tracing::warn!(
            dropped = pending.len(),
            "the routing gate timed out; pending deltas are dropped"
        );
        pending.clear();
    }
    outcome.unwrap_or_else(|_elapsed| {
        tracing::warn!("the routing sync passed its deadline; its detached batch is dropped");
        SyncOutcome::Failed
    })
}

// another saturated callback schedules a successor even if it arrives after this final queue check
async fn push_batches<S>(state: &SyncState<S>) -> SyncOutcome
where
    S: DeltaStore,
{
    loop {
        let outcome = delta::push_and_merge(state).await;
        let pending = state.pending.lock().unwrap_or_else(PoisonError::into_inner);
        if outcome != SyncOutcome::Pushed || pending.len() < MAX_PENDING_DELTAS {
            return outcome;
        }
    }
}
