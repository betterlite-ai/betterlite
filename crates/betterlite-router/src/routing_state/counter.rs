use std::collections::BTreeSet;
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rust_decimal::Decimal;
use tokio::task::JoinSet;

use crate::router_observations::counter::CounterError;
use crate::router_observations::keys::{ROUTER_COUNTER_TTL, outcome_keys};
use crate::routing_value_store::{
    RouteIncrement, RouteKey, RouteValue, RoutingValueStore, StoreRead,
};

use super::Settings;
use super::delta::{DeltaStore, Recorded, SyncState};
use super::error::RoutingStateError;

pub struct RouterCounters<S: DeltaStore + 'static> {
    pub(super) state: Arc<SyncState<S>>,
    pub(super) settings: Settings,
    pub(super) tasks: Mutex<JoinSet<()>>,
}

impl<S: DeltaStore + 'static> RouterCounters<S> {
    #[must_use]
    pub fn new(settings: Settings, local: Arc<RoutingValueStore>, store: Option<Arc<S>>) -> Self {
        Self {
            state: Arc::new(SyncState::new(local, store)),
            settings,
            tasks: Mutex::new(JoinSet::new()),
        }
    }

    // BaseRoutingStrategy._increment_value_in_current_window
    pub async fn increment_window_value(
        &self,
        key: &str,
        value: Decimal,
        ttl: Duration,
    ) -> Result<Decimal, RoutingStateError> {
        let route_key = RouteKey::parse(key)?;
        self.state.track(&route_key);
        let (updated, recorded) = self.state.record(&route_key, value, ttl)?;
        if let Recorded::Saturated(delta) = recorded {
            self.flush_or_drop(delta).await;
        }
        spend_value(&route_key, &updated)
    }

    // BaseRoutingStrategy._increment_value_list_in_current_window
    pub async fn increment_window_values(
        &self,
        increments: &[(String, Decimal)],
        ttl: Duration,
    ) -> Result<Vec<Decimal>, RoutingStateError> {
        let mut results = Vec::with_capacity(increments.len());
        for (key, value) in increments {
            results.push(self.increment_window_value(key, *value, ttl).await?);
        }
        Ok(results)
    }

    // BaseRoutingStrategy.add_to_in_memory_keys_to_update
    pub fn track_key(&self, key: &str) {
        match RouteKey::parse(key) {
            Ok(route_key) => self.state.track(&route_key),
            Err(_source) => tracing::warn!("a routing window key is longer than the key limit"),
        }
    }

    // BaseRoutingStrategy.get_in_memory_keys_to_update
    #[must_use]
    pub fn get_tracked_keys(&self) -> BTreeSet<String> {
        self.state
            .tracked_keys()
            .into_iter()
            .map(|key| key.as_ref().to_owned())
            .collect()
    }

    // BaseRoutingStrategy.get_and_reset_in_memory_keys_to_update
    #[must_use]
    pub fn take_tracked_keys(&self) -> BTreeSet<String> {
        let mut tracked = self
            .state
            .tracked
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        mem::take(&mut *tracked)
            .into_iter()
            .map(|key| key.as_ref().to_owned())
            .collect()
    }

    // BaseRoutingStrategy.reset_in_memory_keys_to_update: the keys go without being handed out
    pub fn reset_tracked_keys(&self) {
        self.state
            .tracked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    // track_deployment_metrics.increment_deployment_successes_for_current_minute
    pub fn increment_successes_for_minute(
        &self,
        deployment_id: &str,
    ) -> Result<String, CounterError> {
        let key = outcome_keys(deployment_id).successes;
        increment_local_counter(self.state.as_ref(), &key, 1, ROUTER_COUNTER_TTL)?;
        Ok(key)
    }

    // track_deployment_metrics.increment_deployment_failures_for_current_minute
    pub fn increment_failures_for_minute(&self, deployment_id: &str) -> Result<(), CounterError> {
        let key = outcome_keys(deployment_id).failures;
        increment_local_counter(self.state.as_ref(), &key, 1, ROUTER_COUNTER_TTL)?;
        Ok(())
    }

    // track_deployment_metrics.get_deployment_successes_for_current_minute
    #[must_use]
    pub fn successes_for_minute(&self, deployment_id: &str) -> i64 {
        let Some(value) =
            local_counter(self.state.as_ref(), &outcome_keys(deployment_id).successes)
        else {
            return 0;
        };
        value
    }

    // track_deployment_metrics.get_deployment_failures_for_current_minute
    #[must_use]
    pub fn failures_for_minute(&self, deployment_id: &str) -> i64 {
        let Some(value) = local_counter(self.state.as_ref(), &outcome_keys(deployment_id).failures)
        else {
            return 0;
        };
        value
    }
}

fn spend_value(key: &RouteKey, value: &RouteValue) -> Decimal {
    match value {
        RouteValue::Spend(spend) => *spend,
        RouteValue::Counter(count) => Decimal::from(*count),
        RouteValue::Pin(_) | RouteValue::SchedulerTuple(_) | RouteValue::Members(_) => {
            tracing::warn!(key = key.as_ref(), "the window value is not numeric");
            Decimal::ZERO
        }
    }
}

// track_deployment_metrics reads a local counter value for the success and failure callbacks
pub(super) fn local_counter<S: DeltaStore>(state: &SyncState<S>, key: &str) -> Option<i64> {
    let route_key = RouteKey::parse(key).ok()?;
    match state.local.get(&route_key) {
        Ok(StoreRead::Value(RouteValue::Counter(value))) => Some(value),
        Ok(StoreRead::Value(RouteValue::Spend(value))) => value.to_i64(),
        Ok(StoreRead::Value(_) | StoreRead::Missing) => None,
        Err(error) => {
            tracing::warn!(%error, "a routing counter could not be read");
            None
        }
    }
}

// LiteLLM's async_increment on the in-memory cache: an existing spend and a counter both add
pub(super) fn increment_local_counter<S: DeltaStore>(
    state: &SyncState<S>,
    key: &str,
    value: i64,
    ttl: Duration,
) -> Result<(), CounterError> {
    let route_key = RouteKey::parse(key).map_err(|_source| CounterError::Rejected {
        key: key.to_owned(),
    })?;
    state
        .local
        .increment(
            &route_key,
            &RouteIncrement::Counter(value),
            Some(ttl.as_secs_f64()),
        )
        .map_err(|_source| CounterError::Rejected {
            key: key.to_owned(),
        })?;
    Ok(())
}
