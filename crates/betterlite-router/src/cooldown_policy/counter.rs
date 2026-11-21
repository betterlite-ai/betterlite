use std::sync::Arc;

use crate::routing_state_store::RoutingStateStore;
use crate::routing_value_store::{RouteIncrement, RouteKey, RouteValue, RoutingValueStore};

// the count one allowed-fails key took, and whether it came from this worker alone
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CounterObservation {
    pub fails: u64,
    pub local_only: bool,
}

// DualCache.increment_cache for the allowed-fails counters: the worker's tier first, then the shared one,
// and a shared failure degrades to this worker's own count instead of failing the cooldown check
pub struct AllowedFailsStore {
    pub local: Arc<RoutingValueStore>,
    pub remote: Option<Arc<RoutingStateStore>>,
}

impl AllowedFailsStore {
    #[must_use]
    pub const fn new(
        local: Arc<RoutingValueStore>,
        remote: Option<Arc<RoutingStateStore>>,
    ) -> Self {
        Self { local, remote }
    }

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
            Ok(observed) => match observed.value {
                RouteValue::Counter(value) => CounterObservation {
                    fails: count(value),
                    local_only: false,
                },
                RouteValue::Spend(_)
                | RouteValue::Pin(_)
                | RouteValue::SchedulerTuple(_)
                | RouteValue::Members(_) => {
                    tracing::warn!(
                        "the allowed-fails key {key} holds a value that is not a counter"
                    );
                    CounterObservation {
                        fails: local,
                        local_only: true,
                    }
                }
            },
            Err(error) => {
                tracing::warn!(
                    "the shared allowed-fails counter for {key} failed, so this worker's count answers: {error}"
                );
                CounterObservation {
                    fails: local,
                    local_only: true,
                }
            }
        }
    }

    fn local_count(&self, key: &str, ttl_seconds: f64) -> Option<u64> {
        let route = RouteKey::parse(key).ok()?;
        match self
            .local
            .increment(&route, &RouteIncrement::Counter(1), Some(ttl_seconds))
        {
            Ok(RouteValue::Counter(value)) => Some(count(value)),
            Ok(
                RouteValue::Spend(_)
                | RouteValue::Pin(_)
                | RouteValue::SchedulerTuple(_)
                | RouteValue::Members(_),
            ) => {
                tracing::warn!("the allowed-fails key {key} holds a value that is not a counter");
                None
            }
            Err(error) => {
                tracing::warn!("the local allowed-fails counter for {key} failed: {error}");
                None
            }
        }
    }
}

fn count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}
