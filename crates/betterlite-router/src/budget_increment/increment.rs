use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rust_decimal::Decimal;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::task::TaskTracker;

use crate::routing_value_store::Clock;

use super::error::IncrementError;
use super::queue::{IncrementQueue, QueuedIncrement};
use super::window::{BudgetConfig, WindowCache};

pub struct BudgetIncrement<C: WindowCache + 'static> {
    pub cache: Arc<C>,
    pub clock: Clock,
    pub queue: Arc<Mutex<IncrementQueue>>,
    pub flush_lock: Arc<AsyncMutex<()>>,
}

impl<C: WindowCache + 'static> BudgetIncrement<C> {
    pub async fn increment_spend_for_key(
        &self,
        budget: &BudgetConfig,
        spend_key: &str,
        start_time_key: &str,
        response_cost: Decimal,
    ) -> Result<(), IncrementError> {
        let Some(duration) = budget.budget_duration else {
            return Ok(());
        };
        let now_seconds = (self.clock)();
        let now = Duration::try_from_secs_f64(now_seconds).map_err(|_source| {
            IncrementError::InvalidSeconds {
                seconds: now_seconds,
            }
        })?;
        let start = self
            .window_start(start_time_key, now_seconds, now, duration)
            .await?;
        let elapsed = now.saturating_sub(start);
        if elapsed > duration {
            self.cache
                .reset_spend(spend_key, response_cost, duration)
                .await?;
            self.cache
                .write_start_time(start_time_key, now_seconds, duration)
                .await?;
            return Ok(());
        }
        let ttl = remaining_ttl(now, start, duration);
        self.cache
            .increment_spend(spend_key, response_cost, ttl)
            .await?;
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .queued
            .push(QueuedIncrement {
                key: spend_key.to_owned(),
                increment_value: response_cost,
                ttl,
            });
        Ok(())
    }

    async fn window_start(
        &self,
        start_time_key: &str,
        now_seconds: f64,
        now: Duration,
        duration: Duration,
    ) -> Result<Duration, IncrementError> {
        if let Some(cached) = self.cache.read_start_time(start_time_key).await? {
            return Duration::try_from_secs_f64(cached)
                .map_err(|_source| IncrementError::InvalidSeconds { seconds: cached });
        }
        self.cache
            .write_start_time(start_time_key, now_seconds, duration)
            .await?;
        Ok(now)
    }

    pub async fn flush_increments(&self, tasks: &TaskTracker) -> bool {
        let cache = Arc::clone(&self.cache);
        let queue = Arc::clone(&self.queue);
        let flush_lock = Arc::clone(&self.flush_lock);
        let flush = tasks.spawn(async move {
            let _guard = flush_lock.lock().await;
            write_queued(cache.as_ref(), &queue).await
        });
        flush.await.unwrap_or_default()
    }
}

// a start cached ahead of this clock extends the first window, as LiteLLM's float subtraction does
fn remaining_ttl(now: Duration, start: Duration, duration: Duration) -> Duration {
    let remaining = start.checked_sub(now).map_or_else(
        || duration.saturating_sub(now.saturating_sub(start)),
        |ahead| duration.saturating_add(ahead),
    );
    // LiteLLM's int(remaining_time) truncates the floating-point seconds
    Duration::from_secs(remaining.as_secs())
}

async fn write_queued<C>(cache: &C, queue: &Mutex<IncrementQueue>) -> bool
where
    C: WindowCache,
{
    let batch = queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .detach();
    if batch.is_empty() {
        queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .detached = None;
        return true;
    }
    if cache.write_increments(&batch).await {
        queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .detached = None;
        return true;
    }
    let _requeued = queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .requeue();
    false
}
