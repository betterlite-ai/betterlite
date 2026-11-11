use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::routing_state_store::RoutingStateStore;
use crate::routing_value_store::{
    RouteIncrement, RouteKey, RouteValue, RoutingValueStore, StoreError,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CountObservation {
    Counted(i64),
    // the stored counter plus the increment does not fit i64, so the add never landed and the true sum is larger
    Overflow,
    Unavailable,
}

#[async_trait]
pub trait IoCounter: Send + Sync {
    async fn increment_counter(&self, key: &str, value: i64, ttl_seconds: u64) -> CountObservation;
}

// DualCache increments the local cache first and keeps that count when Redis fails or is not configured
pub struct DualIoCounter {
    pub local: RoutingValueStore,
    pub remote: Option<Arc<RoutingStateStore>>,
}

impl DualIoCounter {
    // the pinned DualCache returns the local result when the Redis call raises or no Redis client exists
    async fn remote_count(
        &self,
        route: &RouteKey,
        value: i64,
        ttl: f64,
        local: i64,
    ) -> CountObservation {
        let Some(remote) = &self.remote else {
            return CountObservation::Counted(local);
        };
        let outcome = remote
            .increment(route, &RouteIncrement::Counter(value), Some(ttl))
            .await;
        match outcome {
            Ok(observed) => {
                if let Some(warning) = observed.warning {
                    tracing::warn!(?warning, "the io token counter store reported a warning");
                }
                match observed.value {
                    RouteValue::Counter(current) => CountObservation::Counted(current),
                    other @ (RouteValue::Spend(_)
                    | RouteValue::Pin(_)
                    | RouteValue::SchedulerTuple(_)
                    | RouteValue::Members(_)) => {
                        tracing::warn!(
                            ?other,
                            "the remote io token counter holds a non-counter value"
                        );
                        CountObservation::Counted(local)
                    }
                }
            }
            Err(error) => {
                tracing::warn!(%error, "the remote io token counter increment failed; using the local count");
                CountObservation::Counted(local)
            }
        }
    }
}

#[async_trait]
impl IoCounter for DualIoCounter {
    async fn increment_counter(&self, key: &str, value: i64, ttl_seconds: u64) -> CountObservation {
        let route = match RouteKey::parse(key) {
            Ok(route) => route,
            Err(error) => {
                tracing::warn!(%error, "the io token counter key is not usable");
                return CountObservation::Unavailable;
            }
        };
        let ttl = Duration::from_secs(ttl_seconds).as_secs_f64();
        let local = match self
            .local
            .increment(&route, &RouteIncrement::Counter(value), Some(ttl))
        {
            Ok(RouteValue::Counter(current)) => current,
            Ok(other) => {
                tracing::warn!(
                    ?other,
                    "the local io token counter holds a non-counter value"
                );
                return CountObservation::Unavailable;
            }
            Err(StoreError::CounterOverflow { .. }) => return CountObservation::Overflow,
            Err(error) => {
                tracing::warn!(%error, "the local io token counter increment failed");
                return CountObservation::Unavailable;
            }
        };
        self.remote_count(&route, value, ttl, local).await
    }
}
