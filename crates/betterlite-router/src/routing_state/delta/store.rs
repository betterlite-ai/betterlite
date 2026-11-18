use std::collections::BTreeMap;
use std::mem;
use std::sync::PoisonError;

use async_trait::async_trait;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive as _, ToPrimitive as _};

use crate::routing_state_store::RoutingStateStore;
use crate::routing_value_store::{
    RouteIncrement, RouteKey, RouteValue, RoutingValueStore, StoreRead,
};

use super::super::error::RoutingStateError;
use super::{Delta, DeltaStore, SyncState};

#[async_trait]
impl DeltaStore for RoutingStateStore {
    async fn increment_pipeline(
        &self,
        deltas: &[Delta],
    ) -> Result<Vec<Decimal>, RoutingStateError> {
        let mut entries = Vec::with_capacity(deltas.len());
        for delta in deltas {
            let Some(value) = delta.value.to_f64() else {
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

pub(super) async fn send<S: DeltaStore>(
    state: &SyncState<S>,
    batch: &[Delta],
) -> Option<BTreeMap<String, Decimal>> {
    let store = state.store.as_ref()?;
    let compressed = compress(batch);
    match store.increment_pipeline(&compressed).await {
        Ok(values) => Some(values_by_key(&compressed, &values)),
        Err(error) => {
            tracing::warn!(%error, "the delta batch was discarded after a store failure");
            None
        }
    }
}

// the snapshot of what the store push covers: read and detach share the lock the increments take
pub(super) fn snapshot<S: DeltaStore>(
    state: &SyncState<S>,
    keys: &[RouteKey],
) -> Option<(Vec<Decimal>, Vec<Delta>)> {
    let mut pending = state.pending.lock().unwrap_or_else(PoisonError::into_inner);
    let before = keys
        .iter()
        .map(|key| local_amount(&state.local, key))
        .collect();
    let batch = mem::take(&mut *pending);
    if batch.is_empty() {
        return None;
    }
    Some((before, batch))
}

// the merge adds the store's post-push growth to the local value in one update, so an increment racing it is kept
pub(super) fn merge<S: DeltaStore>(
    state: &SyncState<S>,
    keys: &[RouteKey],
    before: &[Decimal],
    values: &BTreeMap<String, Decimal>,
) {
    let _pending = state.pending.lock().unwrap_or_else(PoisonError::into_inner);
    for (key, before) in keys.iter().zip(before) {
        let after = local_amount(&state.local, key);
        let redis_value = values.get(key.as_ref()).copied().unwrap_or_default();
        if after > redis_value {
            continue;
        }
        let adjustment = redis_value.saturating_sub(*before);
        if let Err(error) = state
            .local
            .increment(key, &RouteIncrement::Spend(adjustment), None)
        {
            tracing::warn!(%error, "the routing sync could not write the merged value back");
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

pub(crate) fn local_amount(local: &RoutingValueStore, key: &RouteKey) -> Decimal {
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
