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

use super::DEFAULT_REDIS_SYNC_INTERVAL;
use super::error::BudgetStateError;
use super::event::{CallType, SuccessEvent};
use super::spend_redis::{SpendRead, SpendRedis, set_local_spend, spend_keys};

// the flush is retried at shutdown because a rejected batch stays queued; three attempts cover a store blip
const SHUTDOWN_FLUSH_ATTEMPTS: u8 = 3;

pub struct RoutingBudgetState<R: WindowRedis + SpendRedis + 'static> {
    pub increments: BudgetIncrement<DualWindowCache<R>>,
    pub windows: BudgetWindows,
    pub sync: Mutex<JoinSet<()>>,
    pub flushes: TaskTracker,
}

impl<R: WindowRedis + SpendRedis + 'static> RoutingBudgetState<R> {
    // async_log_success_event: the websocket wrappers spend through their inner calls, then each tier spends once
    pub async fn log_success(&self, event: &SuccessEvent<'_>) -> Result<(), BudgetStateError> {
        match event.call_type {
            CallType::ResponsesWebsocket | CallType::Realtime => return Ok(()),
            CallType::Absent | CallType::Other(_) => {}
        }
        let cost = event.response_cost;
        // only a provider in the configuration opens a window
        if let Some(provider) = event.custom_llm_provider
            && let Some(budget) = self
                .windows
                .provider
                .as_ref()
                .and_then(|configs| configs.get(provider))
        {
            let keys = provider_keys(provider, &budget.period);
            self.increment(keys, budget.budget_limit, budget.period.as_ref(), cost)
                .await?;
        }
        // str(standard_logging_payload.get("model_id", "")): an absent id names no deployment
        let model_id = event.model_id.unwrap_or_default();
        if let Some(budget) = self
            .windows
            .deployment
            .as_ref()
            .and_then(|configs| configs.get(model_id))
        {
            let keys = deployment_keys(model_id, &budget.period);
            self.increment(keys, Some(budget.max_budget), Some(&budget.period), cost)
                .await?;
        }
        self.increment_tags(event.tags, cost).await
    }

    // _sync_in_memory_spend_with_redis: flush first, so the read already holds everything this instance spent
    #[must_use]
    pub async fn sync_with_redis(&self) -> bool {
        let Some(redis) = &self.increments.cache.redis else {
            // no shared store to write to: the flush only clears the queue LiteLLM would leave growing
            return self.increments.flush_increments(&self.flushes).await;
        };
        if !self.increments.flush_increments(&self.flushes).await {
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
    pub async fn shutdown(&self) -> Result<(), BudgetStateError> {
        self.cancel_sync_task();
        let mut sync = mem::take(&mut *self.sync.lock().unwrap_or_else(PoisonError::into_inner));
        while sync.join_next().await.is_some() {}
        for _attempt in 0..SHUTDOWN_FLUSH_ATTEMPTS {
            if self.increments.flush_increments(&self.flushes).await {
                self.flushes.close();
                self.flushes.wait().await;
                return Ok(());
            }
        }
        // the batch is still queued, so a later shutdown can retry it
        let entries = {
            let queue = self
                .increments
                .queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            queue
                .queued
                .len()
                .saturating_add(queue.detached.as_ref().map_or(0, Vec::len))
        };
        Err(BudgetStateError::Shutdown { entries })
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

    // _flush_increments_then_copy_redis_spend: set the memory value to the shared spend plus what is
    // still queued; holding the queue lock across the snapshot and the write keeps an increment that
    // arrives meanwhile in the queue, so the next sync counts it
    async fn copy_redis_spend(&self, redis: &R, keys: &[String]) {
        let SpendRead::Values(values) = redis.read_spends(keys).await else {
            return;
        };
        let queue = self
            .increments
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let pending = pending_increments(&queue);
        for (key, value) in values {
            let Some(redis_spend) = value else {
                continue;
            };
            let spend = redis_spend.saturating_add(pending.get(&key).copied().unwrap_or_default());
            if let Err(source) = set_local_spend(self.increments.cache.as_ref(), &key, spend) {
                tracing::warn!(%key, %source, "the budget spend copy to the memory cache failed");
            }
        }
        drop(queue);
    }
}

// _sum_increments_by_key: the increments the queue still holds, per key
fn pending_increments(queue: &IncrementQueue) -> BTreeMap<String, Decimal> {
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
