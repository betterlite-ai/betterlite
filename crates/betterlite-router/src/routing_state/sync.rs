use std::time::Duration;

use crate::router_observations::counter::Increment;
use crate::routing_value_store::{RouteKey, RouteValue};

use super::FLUSH_DEADLINE;
use super::counter::RouterCounters;
use super::delta::{self, Delta, DeltaStore, local_amount};

impl<S: DeltaStore + 'static> RouterCounters<S> {
    // BaseRoutingStrategy._sync_in_memory_spend_with_redis
    pub async fn sync_with_redis(&self) -> bool {
        if self.state.store.is_none() {
            return false;
        }
        let _flush = self.state.flush_lock.lock().await;
        let keys = self.state.tracked_keys();
        let mut before = Vec::with_capacity(keys.len());
        for key in &keys {
            before.push(local_amount(&self.state.local, key));
        }
        let Some(redis_values) = delta::push_pending(self.state.as_ref()).await else {
            return false;
        };
        for (key, before) in keys.iter().zip(before) {
            let after = local_amount(&self.state.local, key);
            let redis_value = redis_values.get(key.as_ref()).copied().unwrap_or_default();
            if after > redis_value {
                continue;
            }
            let delta = after.saturating_sub(before);
            let merged = redis_value.saturating_add(delta);
            if let Err(error) = self.state.local.set(key, &RouteValue::Spend(merged), None) {
                tracing::warn!(%error, "the routing sync could not write the merged value back");
            }
        }
        true
    }

    // the queue bound is the owner's batch size: a full queue flushes once in the caller's timeframe
    pub(super) async fn queue_window_delta(&self, key: RouteKey, value: i64, ttl: Duration) {
        let delta = Delta { key, value, ttl };
        if self.state.queue(delta.clone()) {
            return;
        }
        let flushed = tokio::time::timeout(FLUSH_DEADLINE, self.sync_with_redis()).await;
        if !matches!(flushed, Ok(true)) {
            tracing::warn!(
                key = delta.key.as_ref(),
                "the queue is full; the delta is dropped"
            );
            return;
        }
        if !self.state.queue(delta) {
            tracing::warn!("the routing delta queue stayed full; the observation is dropped");
        }
    }

    // B397's synchronous CounterSink cannot await redis, so the usage delta joins the owner's batch
    pub(super) fn queue_usage(&self, increment: &Increment) {
        if self.state.store.is_none() {
            return;
        }
        let Ok(route_key) = RouteKey::parse(&increment.key) else {
            tracing::warn!("a usage counter key is longer than the key limit");
            return;
        };
        let delta = Delta {
            key: route_key,
            value: increment.value,
            ttl: increment.ttl,
        };
        if !self.state.queue(delta) {
            tracing::warn!(key = %increment.key, "the queue is full; the usage delta is not stored");
        }
    }
}
