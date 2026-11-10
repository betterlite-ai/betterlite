use std::time::Duration;

use async_trait::async_trait;

use crate::routing_state_store::RoutingStateStore;
use crate::routing_value_store::{RouteIncrement, RouteKey, RouteValue};

#[async_trait]
pub trait IoCounter: Send + Sync {
    // None is the pinned async fallback: an unreachable cache observes no count instead of failing the request
    async fn increment_counter(&self, key: &str, value: i64, ttl_seconds: u64) -> Option<i64>;
}

#[async_trait]
impl IoCounter for RoutingStateStore {
    async fn increment_counter(&self, key: &str, value: i64, ttl_seconds: u64) -> Option<i64> {
        let route = match RouteKey::parse(key) {
            Ok(route) => route,
            Err(error) => {
                tracing::warn!(%error, "the io token counter key is not usable");
                return None;
            }
        };
        let ttl = Duration::from_secs(ttl_seconds).as_secs_f64();
        let outcome =
            Self::increment(self, &route, &RouteIncrement::Counter(value), Some(ttl)).await;
        match outcome {
            Ok(observed) => {
                if let Some(warning) = observed.warning {
                    tracing::warn!(?warning, "the io token counter store reported a warning");
                }
                match observed.value {
                    RouteValue::Counter(current) => Some(current),
                    other @ (RouteValue::Spend(_)
                    | RouteValue::Pin(_)
                    | RouteValue::SchedulerTuple(_)
                    | RouteValue::Members(_)) => {
                        tracing::warn!(?other, "the io token counter holds a non-counter value");
                        None
                    }
                }
            }
            Err(error) => {
                tracing::warn!(%error, "the io token counter increment failed");
                None
            }
        }
    }
}
