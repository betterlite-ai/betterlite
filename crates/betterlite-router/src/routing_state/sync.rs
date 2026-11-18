use std::sync::{Arc, Mutex, PoisonError};

use tokio::runtime::Handle;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::FLUSH_DEADLINE;
use super::delta::{self, DeltaStore, SyncOutcome, SyncState};

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
        if timeout(FLUSH_DEADLINE, drain(&owned)).await.is_err() {
            delta::discard_joined(&owned);
            tracing::warn!("the scheduled routing flush passed its deadline");
        }
    });
}

// one bounded inline flush for an async caller: sync_state takes the gate itself and merges the store's totals
pub(super) async fn flush<S>(state: &SyncState<S>)
where
    S: DeltaStore,
{
    if timeout(FLUSH_DEADLINE, delta::sync_state(state))
        .await
        .is_err()
    {
        tracing::warn!("the routing flush passed its deadline");
    }
}

// push the batch, then push again while a delta that found the queue full joined the running flush
async fn drain<S>(state: &SyncState<S>) -> SyncOutcome
where
    S: DeltaStore,
{
    let mut outcome = delta::push_and_merge(state).await;
    while matches!(outcome, SyncOutcome::Pushed | SyncOutcome::Empty)
        && delta::joined_len(state) > 0
    {
        outcome = delta::push_and_merge(state).await;
    }
    if !matches!(outcome, SyncOutcome::Pushed | SyncOutcome::Empty) {
        delta::discard_joined(state);
    }
    outcome
}
