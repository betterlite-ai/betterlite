pub(crate) mod store;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;
use tokio::sync::Notify;

use crate::routing_value_store::{
    RouteIncrement, RouteKey, RouteValue, RoutingValueStore, StoreError,
};

use self::store::{merge, send, snapshot};
use super::error::RoutingStateError;
use super::{FLUSH_DEADLINE, MAX_PENDING_DELTAS, MAX_TRACKED_KEYS};

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
    pub(super) pending: Mutex<Vec<Delta>>,
    pub(super) tracked: Mutex<BTreeSet<RouteKey>>,
    pub(super) gate: FlushGate,
}

impl<S: DeltaStore> SyncState<S> {
    pub(super) fn new(local: Arc<RoutingValueStore>, store: Option<Arc<S>>) -> Self {
        Self {
            local,
            store,
            pending: Mutex::new(Vec::new()),
            tracked: Mutex::new(BTreeSet::new()),
            gate: FlushGate::new(),
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
    pub(super) fn record_delta(
        &self,
        key: &RouteKey,
        increment: RouteIncrement,
        ttl: Duration,
    ) -> Result<(RouteValue, Option<Delta>), StoreError> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let updated = self
            .local
            .increment(key, &increment, Some(ttl.as_secs_f64()))?;
        if self.store.is_none() {
            return Ok((updated, None));
        }
        let value = match increment {
            RouteIncrement::Spend(value) => value,
            RouteIncrement::Counter(value) => Decimal::from(value),
        };
        let delta = Delta {
            key: key.clone(),
            value,
            ttl,
        };
        if pending.len() < MAX_PENDING_DELTAS {
            pending.push(delta);
            return Ok((updated, None));
        }
        Ok((updated, Some(delta)))
    }

    pub(super) fn queue(&self, delta: Delta) -> bool {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        if pending.len() >= MAX_PENDING_DELTAS {
            return false;
        }
        pending.push(delta);
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
