use std::time::Duration;

use crate::router_observations::counter::{CounterError, CounterSink, Increment};
use crate::routing_value_store::{RouteKey, RouteValue};

use super::counter::{RouterCounters, increment_local_counter, local_counter};
use super::delta::DeltaStore;

impl<S: DeltaStore + 'static> CounterSink for RouterCounters<S> {
    // cache.increment_cache(local_only=True): the success, failure and request counters never reach redis
    fn increment(&self, key: &str, value: i64, ttl: Duration) -> Result<(), CounterError> {
        increment_local_counter(self.state.as_ref(), key, value, ttl)
    }

    // cache.async_increment_cache_pipeline: the local value and its queued delta move together
    fn increment_pipeline(&self, increments: &[Increment]) -> Result<(), CounterError> {
        for increment in increments {
            if let Err(error) = self.queue_usage(increment) {
                tracing::warn!(%error, "a usage counter could not be incremented in memory");
            }
        }
        Ok(())
    }

    fn read(&self, key: &str) -> Option<i64> {
        local_counter(self.state.as_ref(), key)
    }

    fn write(&self, key: &str, value: i64, ttl: Option<Duration>) {
        let Ok(route_key) = RouteKey::parse(key) else {
            tracing::warn!("a routing counter key is longer than the key limit");
            return;
        };
        let seconds = ttl.as_ref().map(Duration::as_secs_f64);
        if let Err(error) = self
            .state
            .local
            .set(&route_key, &RouteValue::Counter(value), seconds)
        {
            tracing::warn!(%error, "a routing counter could not be written");
        }
    }
}
