pub(crate) mod store;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;
use tokio::sync::Notify;

use crate::router_observations::counter::CounterError;
use crate::routing_value_store::{
    RouteIncrement, RouteKey, RouteValue, RoutingValueStore, StoreError,
};

use self::store::{merge, send, snapshot};
use super::error::RoutingStateError;
use super::{FLUSH_DEADLINE, MAX_OVERFLOW_DELTAS, MAX_PENDING_DELTAS, MAX_TRACKED_KEYS};

// one RedisPipelineIncrementOperation: the window delta, its key and the ttl the first write of it carries
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delta {
    pub key: RouteKey,
    pub value: Decimal,
    pub ttl: Duration,
}

#[async_trait]
pub trait DeltaStore: Send + Sync {
    async fn increment_pipeline(&self, deltas: &[Delta])
    -> Result<Vec<Decimal>, RoutingStateError>;
}

// the queue a full caller flushed into, and the buffer a callback's delta waits in when it could not flush
#[derive(Default)]
pub(super) struct Batches {
    pub(super) pending: Vec<Delta>,
    pub(super) overflow: Vec<Delta>,
}

// the owner of the store push: a flag plus a waker, so no lock is held across a storage await
pub(super) struct FlushGate {
    busy: AtomicBool,
    idle: Notify,
}

impl FlushGate {
    pub(super) const fn new() -> Self {
        Self {
            busy: AtomicBool::new(false),
            idle: Notify::const_new(),
        }
    }

    // a caller that cannot get the gate inside its deadline gives up instead of waiting forever
    pub(super) async fn acquire(&self, deadline: Duration) -> Option<FlushGuard<'_>> {
        let start = tokio::time::Instant::now();
        loop {
            if !self.busy.swap(true, Ordering::AcqRel) {
                return Some(FlushGuard { gate: self });
            }
            let Some(remaining) = deadline.checked_sub(start.elapsed()) else {
                return None;
            };
            if tokio::time::timeout(remaining, self.idle.notified())
                .await
                .is_err()
            {
                return None;
            }
        }
    }
}

pub(super) struct FlushGuard<'a> {
    gate: &'a FlushGate,
}

impl Drop for FlushGuard<'_> {
    fn drop(&mut self) {
        self.gate.busy.store(false, Ordering::Release);
        self.gate.idle.notify_one();
    }
}

pub(super) struct SyncState<S: DeltaStore> {
    pub(super) local: Arc<RoutingValueStore>,
    pub(super) store: Option<Arc<S>>,
    pub(super) batches: Mutex<Batches>,
    pub(super) tracked: Mutex<BTreeSet<RouteKey>>,
    pub(super) gate: FlushGate,
    pub(super) flushing_usage: AtomicBool,
}

// what became of the delta an increment queued
pub(super) enum Recorded {
    Queued,
    Saturated(Delta),
}

impl<S: DeltaStore> SyncState<S> {
    pub(super) fn new(local: Arc<RoutingValueStore>, store: Option<Arc<S>>) -> Self {
        Self {
            local,
            store,
            batches: Mutex::new(Batches::default()),
            tracked: Mutex::new(BTreeSet::new()),
            gate: FlushGate::new(),
            flushing_usage: AtomicBool::new(false),
        }
    }

    // add_to_in_memory_keys_to_update: the set is capped so a long-running router never grows it without bound
    pub(super) fn track(&self, key: &RouteKey) {
        let mut tracked = self.tracked.lock().unwrap_or_else(PoisonError::into_inner);
        if tracked.contains(key) {
            return;
        }
        if tracked.len() >= MAX_TRACKED_KEYS {
            tracing::warn!(
                key = key.as_ref(),
                "the update set is full; the key is not tracked"
            );
            return;
        }
        tracked.insert(key.clone());
    }

    pub(super) fn tracked_keys(&self) -> Vec<RouteKey> {
        self.tracked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    // the local value and its queue entry move under one lock, so a sync cannot split an increment in two
    pub(super) fn record(
        &self,
        key: &RouteKey,
        value: Decimal,
        ttl: Duration,
    ) -> Result<(RouteValue, Recorded), StoreError> {
        let mut batches = self.batches.lock().unwrap_or_else(PoisonError::into_inner);
        let updated =
            self.local
                .increment(key, &RouteIncrement::Spend(value), Some(ttl.as_secs_f64()))?;
        if self.store.is_none() {
            return Ok((updated, Recorded::Queued));
        }
        let delta = Delta {
            key: key.clone(),
            value,
            ttl,
        };
        if batches.pending.len() < MAX_PENDING_DELTAS {
            batches.pending.push(delta);
            return Ok((updated, Recorded::Queued));
        }
        Ok((updated, Recorded::Saturated(delta)))
    }

    // the usage increment lands locally first; a full queue hands its delta back to the caller
    pub(super) fn enqueue_usage(
        &self,
        key: &RouteKey,
        value: i64,
        ttl: Duration,
    ) -> Result<Recorded, CounterError> {
        let mut batches = self.batches.lock().unwrap_or_else(PoisonError::into_inner);
        self.local
            .increment(
                key,
                &RouteIncrement::Counter(value),
                Some(ttl.as_secs_f64()),
            )
            .map_err(|_source| CounterError::Rejected {
                key: key.as_ref().to_owned(),
            })?;
        if self.store.is_none() {
            return Ok(Recorded::Queued);
        }
        let delta = Delta {
            key: key.clone(),
            value: Decimal::from(value),
            ttl,
        };
        if batches.pending.len() < MAX_PENDING_DELTAS {
            batches.pending.push(delta);
            return Ok(Recorded::Queued);
        }
        Ok(Recorded::Saturated(delta))
    }

    // the synchronous sink cannot await, so its delta waits in the overflow for the one background flush
    pub(super) fn queue_overflow(&self, delta: Delta) {
        let mut batches = self.batches.lock().unwrap_or_else(PoisonError::into_inner);
        if batches.overflow.len() >= MAX_OVERFLOW_DELTAS {
            tracing::warn!(
                key = delta.key.as_ref(),
                "the usage overflow buffer is full; the delta is not queued"
            );
            return;
        }
        batches.overflow.push(delta);
    }

    pub(super) fn queue(&self, delta: Delta) -> bool {
        let mut batches = self.batches.lock().unwrap_or_else(PoisonError::into_inner);
        if batches.pending.len() >= MAX_PENDING_DELTAS {
            return false;
        }
        batches.pending.push(delta);
        true
    }
}

// BaseRoutingStrategy._sync_in_memory_spend_with_redis: push the queued deltas, then merge the store's totals back
pub(super) async fn sync_state<S: DeltaStore>(state: &SyncState<S>) -> SyncOutcome {
    if state.store.is_none() {
        return SyncOutcome::Empty;
    }
    let Some(_gate) = state.gate.acquire(FLUSH_DEADLINE).await else {
        return SyncOutcome::Busy;
    };
    let keys = state.tracked_keys();
    let Some((before, batch)) = snapshot(state, &keys) else {
        return SyncOutcome::Empty;
    };
    let Some(values) = send(state, &batch).await else {
        return SyncOutcome::Failed;
    };
    merge(state, &keys, &before, &values);
    SyncOutcome::Pushed
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SyncOutcome {
    Pushed,
    Empty,
    Failed,
    Busy,
}

// retire pushes what is queued without merging: the owner is going away, so there is no local value left to update
pub(super) async fn push_pending<S: DeltaStore>(state: &SyncState<S>) -> bool {
    let Some((_before, batch)) = snapshot(state, &state.tracked_keys()) else {
        return false;
    };
    send(state, &batch).await.is_some()
}

// the usage sink cannot await, so a full queue arms one background flush and this drains what it can
pub(super) async fn flush_usage_overflow<S: DeltaStore + 'static>(state: Arc<SyncState<S>>) {
    let _done = UsageFlushDone {
        state: Arc::clone(&state),
    };
    let flushed = tokio::time::timeout(FLUSH_DEADLINE, drain_to_store(state.as_ref())).await;
    if flushed.is_err() {
        tracing::warn!("the routing usage flush did not finish inside its deadline");
    }
}

// the flag clears even when cleanup aborts the task that holds it
struct UsageFlushDone<S: DeltaStore> {
    state: Arc<SyncState<S>>,
}

impl<S: DeltaStore> Drop for UsageFlushDone<S> {
    fn drop(&mut self) {
        self.state.flushing_usage.store(false, Ordering::Release);
    }
}

// drain until a push leaves nothing behind; a failed or busy push leaves its batch to the next sync
async fn drain_to_store<S: DeltaStore>(state: &SyncState<S>) {
    loop {
        if let SyncOutcome::Pushed = sync_state(state).await {
            continue;
        }
        return;
    }
}
