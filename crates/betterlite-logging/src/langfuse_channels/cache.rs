use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::sleep;

use super::{LangfuseTracing, TracingKey};

pub(super) struct Retirement {
    pub(super) id: u64,
    pub(super) task: JoinHandle<()>,
}

pub(super) struct Lease {
    pub(super) tracing: LangfuseTracing,
    pub(super) holders: u32,
    pub(super) retire: Option<Retirement>,
    pub(super) runtime: Handle,
}

pub(super) struct CacheState {
    pub(super) leases: BTreeMap<TracingKey, Lease>,
    pub(super) next_retirement: u64,
}

// A channel owns a batch export thread, so there is one per credential set for the whole process
pub(super) static TRACING: Mutex<CacheState> = Mutex::new(CacheState {
    leases: BTreeMap::new(),
    next_retirement: 0_u64,
});

pub(super) enum ReleaseOutcome {
    Done,
    Shutdown(LangfuseTracing),
}

pub(super) fn insert(
    state: &mut CacheState,
    key: TracingKey,
    tracing: &LangfuseTracing,
    runtime: &Handle,
) {
    state.leases.insert(
        key,
        Lease {
            tracing: tracing.clone(),
            holders: 1,
            retire: None,
            runtime: runtime.clone(),
        },
    );
}

pub(super) fn channels() -> Vec<LangfuseTracing> {
    let state = TRACING.lock().unwrap_or_else(PoisonError::into_inner);
    state
        .leases
        .values()
        .map(|lease| lease.tracing.clone())
        .collect()
}

pub(super) fn begin_release(tracing: &LangfuseTracing, grace: Duration) -> ReleaseOutcome {
    let mut state = TRACING.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(key) = state
        .leases
        .iter()
        .find(|(_, lease)| Arc::ptr_eq(&lease.tracing.ledger.exporter, &tracing.ledger.exporter))
        .map(|(key, _)| key.clone())
    else {
        return ReleaseOutcome::Done;
    };
    let holders = match state.leases.get(&key) {
        Some(lease) => lease.holders,
        None => return ReleaseOutcome::Done,
    };
    if holders == 0 {
        return ReleaseOutcome::Done;
    }
    if holders > 1 {
        if let Some(lease) = state.leases.get_mut(&key) {
            lease.holders = holders.saturating_sub(1);
        }
        return ReleaseOutcome::Done;
    }
    if grace.is_zero() {
        return match state.leases.remove(&key) {
            Some(released) => ReleaseOutcome::Shutdown(released.tracing),
            None => ReleaseOutcome::Done,
        };
    }
    schedule_retirement(&mut state, key, grace);
    drop(state);
    ReleaseOutcome::Done
}

fn schedule_retirement(state: &mut CacheState, key: TracingKey, grace: Duration) {
    let id = state.next_retirement;
    state.next_retirement = id.wrapping_add(1);
    let Some(lease) = state.leases.get_mut(&key) else {
        return;
    };
    let task = lease.runtime.spawn(retire_unless_reacquired(
        key,
        id,
        lease.tracing.clone(),
        grace,
    ));
    lease.holders = 0;
    lease.retire = Some(Retirement { id, task });
}

pub(super) async fn retire_unless_reacquired(
    key: TracingKey,
    id: u64,
    tracing: LangfuseTracing,
    grace: Duration,
) {
    sleep(grace).await;
    if !take_retired(&key, id) {
        return;
    }
    let mut shutdown = JoinSet::new();
    shutdown.spawn_blocking(move || tracing.shutdown());
    let _finished = shutdown.join_next().await;
}

fn take_retired(key: &TracingKey, id: u64) -> bool {
    let mut state = TRACING.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(lease) = state.leases.get(key) else {
        return false;
    };
    if lease.retire.as_ref().is_none_or(|retire| retire.id != id) {
        return false;
    }
    state.leases.remove(key).is_some()
}
