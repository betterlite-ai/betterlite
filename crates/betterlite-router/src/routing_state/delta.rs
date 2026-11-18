pub(crate) mod store;

use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use tokio::time::timeout;

use crate::routing_value_store::{
    RouteIncrement, RouteKey, RouteValue, RoutingValueStore, StoreError,
};

use self::store::{merge, send, snapshot};
use super::error::RoutingStateError;
use super::{MAX_PENDING_DELTAS, MAX_TRACKED_KEYS};

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

// the owner of the store push: an async lock, safe to hold across the storage await, released on drop
pub(super) struct FlushGate {
    pub(super) held: Arc<AsyncMutex<()>>,
}

impl FlushGate {
    pub(super) fn new() -> Self {
        Self {
            held: Arc::new(AsyncMutex::new(())),
        }
    }

    // a caller that cannot get the gate inside its deadline gives up instead of waiting forever
    pub(super) async fn acquire(gate: &Self, deadline: Duration) -> Option<OwnedMutexGuard<()>> {
        timeout(deadline, Arc::clone(&gate.held).lock_owned())
            .await
            .ok()
    }
}

pub(super) struct SyncState<S: DeltaStore> {
    pub(super) local: Arc<RoutingValueStore>,
    pub(super) store: Option<Arc<S>>,
    pub(super) pending: Mutex<Vec<Delta>>,
    pub(super) flush_scheduled: AtomicBool,
    pub(super) tracked: Mutex<BTreeSet<RouteKey>>,
    pub(super) gate: FlushGate,
}

impl<S: DeltaStore> SyncState<S> {
    pub(super) fn new(local: Arc<RoutingValueStore>, store: Option<Arc<S>>) -> Self {
        Self {
            local,
            store,
            pending: Mutex::new(Vec::new()),
            flush_scheduled: AtomicBool::new(false),
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
        increment: &RouteIncrement,
        ttl: Duration,
    ) -> Result<(RouteValue, bool), StoreError> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let updated = self
            .local
            .increment(key, increment, Some(ttl.as_secs_f64()))?;
        let saturated = pending.len() >= MAX_PENDING_DELTAS;
        if self.store.is_some() {
            let value = match increment {
                RouteIncrement::Spend(value) => *value,
                RouteIncrement::Counter(value) => Decimal::from(*value),
            };
            if pending.len() < MAX_PENDING_DELTAS.saturating_mul(2) {
                pending.push(Delta {
                    key: key.clone(),
                    value,
                    ttl,
                });
            } else {
                tracing::warn!(
                    key = key.as_ref(),
                    "the routing delta queue is full; the observation is dropped"
                );
            }
        }
        drop(pending);
        Ok((updated, saturated))
    }
}

// the push and the read-back merge, for a caller that already holds the gate
pub(super) async fn push_and_merge<S>(state: &SyncState<S>) -> SyncOutcome
where
    S: DeltaStore,
{
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
pub(super) async fn push_pending<S>(state: &SyncState<S>) -> bool
where
    S: DeltaStore,
{
    let Some((_before, batch)) = snapshot(state, &state.tracked_keys()) else {
        return false;
    };
    send(state, &batch).await.is_some()
}
