use std::sync::{Arc, Mutex, PoisonError};

use tokio::runtime::Handle;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::FLUSH_DEADLINE;
use super::delta::{self, Delta, DeltaStore, FlushGate, SyncOutcome, SyncState};

// the sink's saturated path: the owner's task set runs the flush, so a stalled store never blocks the callback
pub(super) fn schedule_flush<S>(
    state: &Arc<SyncState<S>>,
    flushes: &Mutex<JoinSet<()>>,
    delta: Delta,
) where
    S: DeltaStore + 'static,
{
    if Handle::try_current().is_err() {
        tracing::warn!(
            key = delta.key.as_ref(),
            "the usage delta needs a running runtime to flush; it is dropped"
        );
        return;
    }
    let Some(gate) = FlushGate::try_acquire(&state.gate) else {
        tracing::warn!(
            key = delta.key.as_ref(),
            "a flush is already running; the delta is dropped"
        );
        return;
    };
    let owned = Arc::clone(state);
    let mut tasks = flushes.lock().unwrap_or_else(PoisonError::into_inner);
    while tasks.try_join_next().is_some() {}
    let _task = tasks.spawn(async move {
        let _guard = gate;
        flush(&owned, delta, true).await;
    });
}

// one bounded flush: a scheduled task holds the gate already, and the async caller waits for it inside sync_state
pub(super) async fn flush<S>(state: &SyncState<S>, delta: Delta, gate_held: bool)
where
    S: DeltaStore,
{
    let outcome = if gate_held {
        timeout(FLUSH_DEADLINE, delta::push_and_merge(state)).await
    } else {
        timeout(FLUSH_DEADLINE, delta::sync_state(state)).await
    };
    finish_flush(state, delta, outcome.unwrap_or(SyncOutcome::Busy));
}

// a delta that found the queue full goes back only after a flush that finished, and only if there is room again
fn finish_flush<S>(state: &SyncState<S>, delta: Delta, outcome: SyncOutcome)
where
    S: DeltaStore,
{
    let key = delta.key.clone();
    if matches!(outcome, SyncOutcome::Pushed | SyncOutcome::Empty) && state.queue(delta) {
        return;
    }
    tracing::warn!(
        key = key.as_ref(),
        "the queue is full; the delta is dropped after the bounded flush"
    );
}
