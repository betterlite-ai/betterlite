use std::collections::BTreeSet;
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use tokio::task::JoinSet;

use crate::router_observations::counter::{CounterError, Increment};
use crate::router_observations::keys::{ROUTER_COUNTER_TTL, outcome_keys};
use crate::routing_value_store::{
    RouteIncrement, RouteKey, RouteValue, RoutingValueStore, StoreRead,
};

use super::delta::{self, DeltaStore, FlushGate, SyncOutcome, SyncState};
use super::error::RoutingStateError;
use super::{FLUSH_DEADLINE, Settings, lifecycle, sync};

pub struct RouterCounters<S: DeltaStore + 'static> {
    pub(super) state: Arc<SyncState<S>>,
    pub(super) settings: Settings,
    pub(super) tasks: Mutex<JoinSet<()>>,
    pub(super) flushes: Mutex<JoinSet<()>>,
}

impl<S: DeltaStore + 'static> RouterCounters<S> {
    #[must_use]
    pub fn new(settings: Settings, local: Arc<RoutingValueStore>, store: Option<Arc<S>>) -> Self {
        Self {
            state: Arc::new(SyncState::new(local, store)),
            settings,
            tasks: Mutex::new(JoinSet::new()),
            flushes: Mutex::new(JoinSet::new()),
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
        let (updated, saturated) =
            self.state
                .record_delta(&route_key, &RouteIncrement::Spend(value), ttl)?;
        if saturated {
            let _outcome = sync::sync_state(self.state.as_ref(), false).await;
        }
        Ok(spend_value(&route_key, &updated))
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

    // B397's sink cannot await the store, so saturation schedules a flush after staging the callback
    pub(super) fn stage_usage(&self, increment: &Increment) -> Result<bool, CounterError> {
        let route_key =
            RouteKey::parse(&increment.key).map_err(|_source| CounterError::Rejected {
                key: increment.key.clone(),
            })?;
        if self.state.store.is_some() {
            self.state.track(&route_key);
        }
        let recorded = self.state.record_delta(
            &route_key,
            &RouteIncrement::Counter(increment.value),
            increment.ttl,
        );
        match recorded {
            Ok((_updated, saturated)) => Ok(saturated),
            Err(_source) => Err(CounterError::Rejected {
                key: increment.key.clone(),
            }),
        }
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

    // BaseRoutingStrategy._sync_in_memory_spend_with_redis
    pub async fn sync_with_redis(&self) -> bool {
        matches!(
            sync::sync_state(self.state.as_ref(), false).await,
            SyncOutcome::Pushed
        )
    }

    // BaseRoutingStrategy.__init__ starts the periodic sync only for a strategy that batches its writes
    pub fn start(self: &Arc<Self>) {
        if self.settings.should_batch_redis_writes {
            self.setup_sync_task();
        }
    }

    // BaseRoutingStrategy.setup_sync_task
    pub fn setup_sync_task(self: &Arc<Self>) {
        let interval = self.settings.resolved_sync_interval();
        let owner = Arc::downgrade(self);
        let mut tasks = self.tasks.lock().unwrap_or_else(PoisonError::into_inner);
        tasks.abort_all();
        while tasks.try_join_next().is_some() {}
        let _task = tasks.spawn(lifecycle::run_periodic(owner, interval));
    }

    // BaseRoutingStrategy.cancel_sync_task
    pub fn cancel_sync_task(&self) {
        self.tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .abort_all();
    }

    // BaseRoutingStrategy.cleanup cancels the sync, then waits for the tasks and the scheduled flushes
    pub async fn cleanup(&self) {
        self.cancel_sync_task();
        let mut tasks = mem::take(&mut *self.tasks.lock().unwrap_or_else(PoisonError::into_inner));
        while tasks.join_next().await.is_some() {}
        let mut flushes =
            mem::take(&mut *self.flushes.lock().unwrap_or_else(PoisonError::into_inner));
        while flushes.join_next().await.is_some() {}
    }

    // BaseRoutingStrategy.retire stops the loop, then pushes what is queued inside the flush deadline
    pub async fn retire(&self) {
        self.cleanup().await;
        let Some(_gate) = FlushGate::acquire(&self.state.gate, FLUSH_DEADLINE).await else {
            tracing::warn!("a routing flush held the store past the retire deadline");
            return;
        };
        let _pushed = delta::push_pending(self.state.as_ref()).await;
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
pub(super) fn local_counter<S>(state: &SyncState<S>, key: &str) -> Option<i64>
where
    S: DeltaStore,
{
    let route_key = RouteKey::parse(key).ok()?;
    match state.local.get(&route_key) {
        Ok(StoreRead::Value(RouteValue::Counter(value))) => Some(value),
        Ok(StoreRead::Value(RouteValue::Spend(value))) if value.fract().is_zero() => value.to_i64(),
        Ok(StoreRead::Value(_) | StoreRead::Missing) => None,
        Err(error) => {
            tracing::warn!(%error, "a routing counter could not be read");
            None
        }
    }
}

// LiteLLM's async_increment on the in-memory cache: an existing spend and a counter both add
pub(super) fn increment_local_counter<S>(
    state: &SyncState<S>,
    key: &str,
    value: i64,
    ttl: Duration,
) -> Result<(), CounterError>
where
    S: DeltaStore,
{
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
