use std::sync::Arc;

use crate::routing_state_store::RoutingStateStore;
use crate::routing_state_store::error::StateStoreError;
use crate::routing_value_store::{RouteIncrement, RouteKey, RouteValue, RoutingValueStore};

// the count one allowed-fails key took, and whether it came from this worker alone
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CounterObservation {
    pub fails: u64,
    pub local_only: bool,
}

// DualCache.increment_cache for the allowed-fails counters: the worker's tier first, then the shared one
pub struct AllowedFailsStore {
    pub local: Arc<RoutingValueStore>,
    pub remote: Option<Arc<RoutingStateStore>>,
}

impl AllowedFailsStore {
    // _increment_allowed_fails: RedisCache.increment_cache raises after bumping the local tier, and the handler then reads that tier
    pub async fn increment(&self, key: &str, ttl_seconds: f64) -> CounterObservation {
        let Some(local) = self.local_count(key, ttl_seconds) else {
            return CounterObservation {
                fails: 0,
                local_only: true,
            };
        };
        let Some(remote) = &self.remote else {
            return CounterObservation {
                fails: local,
                local_only: false,
            };
        };
        let Ok(route) = RouteKey::parse(key) else {
            return CounterObservation {
                fails: local,
                local_only: true,
            };
        };
        match remote
            .increment(&route, &RouteIncrement::Counter(1), Some(ttl_seconds))
            .await
        {
            Ok(observed) => match counter_of(key, &observed.value) {
                Some(fails) => CounterObservation {
                    fails,
                    local_only: false,
                },
                None => CounterObservation {
                    fails: local,
                    local_only: true,
                },
            },
            Err(error) => shared_error(key, local, &error),
        }
    }

    fn local_count(&self, key: &str, ttl_seconds: f64) -> Option<u64> {
        let route = RouteKey::parse(key).ok()?;
        match self
            .local
            .increment(&route, &RouteIncrement::Counter(1), Some(ttl_seconds))
        {
            Ok(value) => counter_of(key, &value),
            Err(error) => {
                tracing::warn!("the local allowed-fails counter for {key} failed: {error}");
                None
            }
        }
    }
}

// a counter entry's count, or the warning a key that holds another value type earns
fn counter_of(key: &str, value: &RouteValue) -> Option<u64> {
    match value {
        RouteValue::Counter(count) => Some(u64::try_from(*count).unwrap_or(0)),
        RouteValue::Spend(_)
        | RouteValue::Pin(_)
        | RouteValue::SchedulerTuple(_)
        | RouteValue::Members(_) => {
            tracing::warn!("the allowed-fails key {key} holds a value that is not a counter");
            None
        }
    }
}

// RedisCache.increment_cache raised after the worker tier took the bump, so that count answers
fn shared_error(key: &str, local: u64, error: &StateStoreError) -> CounterObservation {
    tracing::warn!(
        "the shared allowed-fails counter for {key} failed, so this worker's count answers: {error}"
    );
    CounterObservation {
        fails: local,
        local_only: true,
    }
}
