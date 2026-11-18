use std::collections::BTreeMap;
use std::mem;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use rust_decimal::Decimal;
use tokio::runtime::Handle;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tokio_util::task::TaskTracker;

use crate::budget_increment::dual::DualWindowCache;
use crate::budget_increment::increment::BudgetIncrement;
use crate::budget_increment::queue::IncrementQueue;
use crate::budget_increment::window::{BudgetConfig, WindowRedis};
use crate::budget_windows::budget::BudgetPeriod;
use crate::budget_windows::keys::{WindowKeys, deployment_keys, provider_keys, tag_keys};
use crate::budget_windows::service::BudgetWindows;
use crate::routing_value_store::Clock;

use super::error::BudgetStateError;
use super::event::SuccessEvent;
use super::spend_redis::{SpendRead, SpendRedis, set_local_spend, spend_keys};
use super::{DEFAULT_REDIS_SYNC_INTERVAL, SKIPPED_CALL_TYPES};

pub struct RoutingBudgetState<R: WindowRedis + SpendRedis + 'static> {
    pub increments: BudgetIncrement<DualWindowCache<R>>,
    pub windows: BudgetWindows,
    pub(super) sync: Mutex<JoinSet<()>>,
    flushes: TaskTracker,
}

impl<R: WindowRedis + SpendRedis + 'static> RoutingBudgetState<R> {
    #[must_use]
    pub fn new(windows: BudgetWindows, cache: Arc<DualWindowCache<R>>, clock: Clock) -> Self {
        Self {
            increments: BudgetIncrement {
                cache,
                clock,
                queue: Arc::default(),
                flush_lock: Arc::default(),
            },
            windows,
            sync: Mutex::new(JoinSet::new()),
            flushes: TaskTracker::new(),
        }
    }

    // async_log_success_event: the websocket wrappers spend through their inner calls, then each tier spends once
    pub async fn log_success(&self, event: &SuccessEvent<'_>) -> Result<(), BudgetStateError> {
        if event
            .call_type
            .is_some_and(|call_type| SKIPPED_CALL_TYPES.contains(&call_type))
        {
            return Ok(());
        }
        let cost = event.response_cost;
        if let Some(provider) = event.custom_llm_provider {
            self.increment_provider(provider, cost).await?;
        }
        // str(standard_logging_payload.get("model_id", "")): an absent id names no deployment
        self.increment_deployment(event.model_id.unwrap_or_default(), cost)
            .await?;
        self.increment_tags(event.tags, cost).await
    }

    // _push_in_memory_increments_to_redis: write what the queue holds without copying the shared spends back
    #[must_use]
    pub async fn flush(&self) -> bool {
        self.increments.flush_increments(&self.flushes).await
    }

    // _sync_in_memory_spend_with_redis: flush first, so the read already holds everything this instance spent
    #[must_use]
    pub async fn sync_with_redis(&self) -> bool {
        let Some(redis) = &self.increments.cache.redis else {
            // no shared store to write to: the flush only clears the queue LiteLLM would leave growing
            return self.flush().await;
        };
        if !self.flush().await {
            return false;
        }
        // the flush lock keeps another flush from writing between the read and the copy
        let _flushing = self.increments.flush_lock.lock().await;
        let keys = spend_keys(&self.windows);
        self.copy_redis_spend(redis, &keys).await;
        true
    }

    // periodic_sync_in_memory_spend_with_redis starts through RouterBudgetLimiting.__init__
    pub fn start(self: &Arc<Self>) {
        if Handle::try_current().is_err() {
            tracing::warn!("the budget sync needs a running runtime; it was not started");
            return;
        }
        let owner = Arc::downgrade(self);
        let mut sync = self.sync.lock().unwrap_or_else(PoisonError::into_inner);
        sync.abort_all();
        while sync.try_join_next().is_some() {}
        let _task = sync.spawn(run_periodic(owner));
    }

    pub fn cancel_sync_task(&self) {
        self.sync
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .abort_all();
    }

    // no LiteLLM counterpart: the server stops the periodic task, then spends what the queue still holds
    pub async fn shutdown(&self) {
        self.cancel_sync_task();
        let mut sync = mem::take(&mut *self.sync.lock().unwrap_or_else(PoisonError::into_inner));
        while sync.join_next().await.is_some() {}
        let _flushed = self.flush().await;
        self.flushes.close();
        self.flushes.wait().await;
    }

    // async_log_success_event's provider tier: only a provider in the configuration opens a window
    async fn increment_provider(
        &self,
        provider: &str,
        cost: Decimal,
    ) -> Result<(), BudgetStateError> {
        let Some(budget) = self
            .windows
            .provider
            .as_ref()
            .and_then(|configs| configs.get(provider))
        else {
            return Ok(());
        };
        let keys = provider_keys(provider, &budget.period);
        self.increment(keys, budget.budget_limit, budget.period.as_ref(), cost)
            .await
    }

    // _get_budget_config_for_deployment: the payload's model id names the deployment window
    async fn increment_deployment(
        &self,
        model_id: &str,
        cost: Decimal,
    ) -> Result<(), BudgetStateError> {
        let Some(budget) = self
            .windows
            .deployment
            .as_ref()
            .and_then(|configs| configs.get(model_id))
        else {
            return Ok(());
        };
        let keys = deployment_keys(model_id, &budget.period);
        self.increment(keys, Some(budget.max_budget), Some(&budget.period), cost)
            .await
    }

    // the tag tier spends once per entry of the request's tags, so a tag named twice is counted twice
    async fn increment_tags(&self, tags: &[String], cost: Decimal) -> Result<(), BudgetStateError> {
        let Some(configs) = self.windows.tag.as_ref() else {
            return Ok(());
        };
        for tag in tags {
            let Some(budget) = configs.get(tag) else {
                continue;
            };
            let keys = tag_keys(tag, &budget.period);
            self.increment(keys, budget.max_budget, budget.period.as_ref(), cost)
                .await?;
        }
        Ok(())
    }

    // _increment_spend_for_key builds its increment from the configured duration, never from the max budget
    async fn increment(
        &self,
        keys: WindowKeys,
        max_budget: Option<Decimal>,
        period: Option<&BudgetPeriod>,
        response_cost: Decimal,
    ) -> Result<(), BudgetStateError> {
        let config = BudgetConfig {
            max_budget,
            budget_duration: period.map(|period| period.seconds),
        };
        self.increments
            .increment_spend_for_key(&config, &keys.spend, &keys.start_time, response_cost)
            .await
            .map_err(|source| BudgetStateError::Increment {
                key: keys.spend,
                source,
            })
    }

    // _flush_increments_then_copy_redis_spend: set the memory value to the shared spend plus what is still queued
    async fn copy_redis_spend(&self, redis: &R, keys: &[String]) {
        let SpendRead::Values(values) = redis.read_spends(keys).await else {
            return;
        };
        let pending = pending_increments(self.increments.queue.as_ref());
        for (key, value) in values {
            let Some(redis_spend) = value else {
                continue;
            };
            let spend = redis_spend.saturating_add(pending.get(&key).copied().unwrap_or_default());
            if let Err(source) = set_local_spend(self.increments.cache.as_ref(), &key, spend) {
                tracing::warn!(%key, %source, "the budget spend copy to the memory cache failed");
            }
        }
    }
}

// _sum_increments_by_key: the increments the queue still holds, per key
fn pending_increments(queue: &Mutex<IncrementQueue>) -> BTreeMap<String, Decimal> {
    let queue = queue.lock().unwrap_or_else(PoisonError::into_inner);
    let mut totals = BTreeMap::new();
    for operation in &queue.queued {
        let total = totals.entry(operation.key.clone()).or_default();
        *total = total.saturating_add(operation.increment_value);
    }
    totals
}

// periodic_sync_in_memory_spend_with_redis: one sync, then the fixed wait, whether or not the sync worked
async fn run_periodic<R: WindowRedis + SpendRedis + 'static>(owner: Weak<RoutingBudgetState<R>>) {
    while let Some(state) = owner.upgrade() {
        let _synced = state.sync_with_redis().await;
        drop(state);
        sleep(DEFAULT_REDIS_SYNC_INTERVAL).await;
    }
}
