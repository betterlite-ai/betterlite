use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_router::budget_increment::error::IncrementError;
use betterlite_router::budget_increment::queue::QueuedIncrement;
use betterlite_router::budget_increment::window::WindowRedis;
use betterlite_router::routing_budget_state::spend_redis::{SpendRead, SpendRedis};
use rust_decimal::Decimal;
use tokio::sync::Notify;

// the Redis the limiter talks to: INCRBYFLOAT per key, one float per key like the redis wire
#[derive(Clone, Default)]
pub(super) struct ProbeStore {
    spends: Arc<Mutex<BTreeMap<String, f64>>>,
    starts: Arc<Mutex<BTreeMap<String, f64>>>,
    fail: Arc<AtomicBool>,
    hold: Arc<AtomicBool>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

impl ProbeStore {
    pub(super) fn spend(&self, key: &str) -> Option<f64> {
        self.spends
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .copied()
    }

    pub(super) fn set_fail(&self, fail: bool) {
        self.fail.store(fail, Ordering::SeqCst);
    }

    pub(super) fn set_hold(&self, hold: bool) {
        self.hold.store(hold, Ordering::SeqCst);
    }

    pub(super) fn holding(&self) -> bool {
        self.hold.load(Ordering::SeqCst)
    }

    pub(super) fn entered(&self) -> &Notify {
        &self.entered
    }

    pub(super) fn release_writes(&self) {
        self.release.notify_waiters();
    }
}

#[async_trait]
impl WindowRedis for ProbeStore {
    async fn read_start_time(&self, key: &str) -> Result<Option<f64>, IncrementError> {
        let starts = self.starts.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(starts.get(key).copied())
    }

    async fn write_start_time(&self, key: &str, start_time: f64, _ttl: Duration) {
        self.starts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), start_time);
    }

    async fn write_spend(&self, key: &str, cost: Decimal, _ttl: Duration) {
        self.spends
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), cost.as_f64());
    }

    async fn write_increments(&self, operations: &[QueuedIncrement]) -> bool {
        if self.holding() {
            self.entered.notify_one();
            self.release.notified().await;
        }
        if self.fail.load(Ordering::SeqCst) {
            return false;
        }
        let mut spends = self.spends.lock().unwrap_or_else(PoisonError::into_inner);
        let mut working = spends.clone();
        for operation in operations {
            let current = working.get(&operation.key).copied().unwrap_or_default();
            let total: f64 = [current, operation.increment_value.as_f64()]
                .into_iter()
                .sum();
            if !total.is_finite() {
                return false;
            }
            working.insert(operation.key.clone(), total);
        }
        *spends = working;
        true
    }
}

#[async_trait]
impl SpendRedis for ProbeStore {
    async fn read_spends(&self, keys: &[String]) -> SpendRead {
        SpendRead::Values(
            keys.iter()
                .map(|key| (key.clone(), self.spend_value(key)))
                .collect(),
        )
    }
}

impl ProbeStore {
    fn spend_value(&self, key: &str) -> Option<Decimal> {
        let spend = self.spend(key)?;
        Decimal::from_str(&spend.to_string()).ok()
    }
}

// the float the store answers with, rendered the way the Python probe renders the same float
pub(super) fn render(spend: f64) -> String {
    Decimal::from_str(&spend.to_string()).map_or_else(
        |_source| spend.to_string(),
        |spend| spend.normalize().to_string(),
    )
}
