use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive as _, ToPrimitive as _};
use tokio::sync::Mutex as AsyncMutex;

use crate::routing_state_store::RoutingStateStore;
use crate::routing_value_store::{RouteKey, RouteValue, RoutingValueStore, StoreRead};

use super::error::RoutingStateError;
use super::{MAX_PENDING_DELTAS, MAX_TRACKED_KEYS};

// one RedisPipelineIncrementOperation: the window delta, its key and the ttl the first write of it carries
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delta {
    pub key: RouteKey,
    pub value: i64,
    pub ttl: Duration,
}

#[async_trait]
pub trait DeltaStore: Send + Sync {
    async fn increment_pipeline(&self, deltas: &[Delta])
    -> Result<Vec<Decimal>, RoutingStateError>;
}

#[async_trait]
impl DeltaStore for RoutingStateStore {
    async fn increment_pipeline(
        &self,
        deltas: &[Delta],
    ) -> Result<Vec<Decimal>, RoutingStateError> {
        let mut entries = Vec::with_capacity(deltas.len());
        for delta in deltas {
            let Some(value) = Decimal::from(delta.value).to_f64() else {
                return Err(RoutingStateError::NonFinite);
            };
            entries.push((delta.key.clone(), value, Some(delta.ttl.as_secs_f64())));
        }
        let observed = self.increment_float_pipeline(&entries).await?;
        if let Some(warning) = observed.warning {
            tracing::warn!(
                ?warning,
                "the routing store reported a warning while pushing deltas"
            );
        }
        let Some(values) = observed.value else {
            return Ok(Vec::new());
        };
        let mut decimals = Vec::with_capacity(values.len());
        for value in values {
            let Some(decimal) = Decimal::from_f64(value) else {
                return Err(RoutingStateError::NonFinite);
            };
            decimals.push(decimal);
        }
        Ok(decimals)
    }
}

pub(super) struct SyncState<S: DeltaStore> {
    pub(super) local: Arc<RoutingValueStore>,
    pub(super) store: Option<Arc<S>>,
    pub(super) pending: Mutex<Vec<Delta>>,
    pub(super) tracked: Mutex<BTreeSet<RouteKey>>,
    pub(super) flush_lock: AsyncMutex<()>,
}

impl<S: DeltaStore> SyncState<S> {
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

    pub(super) fn queue(&self, delta: Delta) -> bool {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        if pending.len() >= MAX_PENDING_DELTAS {
            return false;
        }
        pending.push(delta);
        true
    }
}

// BaseRoutingStrategy._push_in_memory_increments_to_redis: compress one batch by key, send it, discard it
pub(super) async fn push_pending<S: DeltaStore>(
    state: &SyncState<S>,
) -> Option<BTreeMap<String, Decimal>> {
    let store = state.store.as_ref()?;
    let batch = {
        let mut pending = state.pending.lock().unwrap_or_else(PoisonError::into_inner);
        mem::take(&mut *pending)
    };
    if batch.is_empty() {
        return None;
    }
    let compressed = compress(&batch);
    match store.increment_pipeline(&compressed).await {
        Ok(values) => Some(values_by_key(&compressed, &values)),
        Err(error) => {
            tracing::warn!(%error, "the delta batch was discarded after a store failure");
            None
        }
    }
}

// the compressed op keeps the first operation's ttl, and later values for the same key add into it
fn compress(batch: &[Delta]) -> Vec<Delta> {
    let mut compressed: Vec<Delta> = Vec::with_capacity(batch.len());
    let mut positions: BTreeMap<&str, usize> = BTreeMap::new();
    for delta in batch {
        if let Some(position) = positions.get(delta.key.as_ref()) {
            if let Some(entry) = compressed.get_mut(*position) {
                entry.value = entry.value.saturating_add(delta.value);
            }
            continue;
        }
        positions.insert(delta.key.as_ref(), compressed.len());
        compressed.push(delta.clone());
    }
    compressed
}

// zip stops at the shorter side, as LiteLLM's zip over the replies does; a missing reply counts as zero
fn values_by_key(compressed: &[Delta], values: &[Decimal]) -> BTreeMap<String, Decimal> {
    compressed
        .iter()
        .zip(values)
        .map(|(delta, value)| (delta.key.as_ref().to_owned(), *value))
        .collect()
}

pub(super) fn local_amount(local: &RoutingValueStore, key: &RouteKey) -> Decimal {
    match local.get(key) {
        Ok(StoreRead::Value(RouteValue::Spend(value))) => value,
        Ok(StoreRead::Value(RouteValue::Counter(value))) => Decimal::from(value),
        Ok(StoreRead::Value(_) | StoreRead::Missing) => Decimal::ZERO,
        Err(error) => {
            tracing::warn!(%error, "a routing counter could not be read back");
            Decimal::ZERO
        }
    }
}
