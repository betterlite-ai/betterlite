use std::collections::BTreeMap;
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

// the shared store the limiter writes to; it keeps the decimal, so a spend reads back as the sum it built
#[derive(Clone, Default)]
pub(super) struct ProbeStore {
    pub(super) spends: Arc<Mutex<BTreeMap<String, Decimal>>>,
    pub(super) starts: Arc<Mutex<BTreeMap<String, f64>>>,
    pub(super) fail: Arc<AtomicBool>,
    pub(super) hold: Arc<AtomicBool>,
    pub(super) entered: Arc<Notify>,
    pub(super) release: Arc<Notify>,
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
            .insert(key.to_owned(), cost);
    }

    async fn write_increments(&self, operations: &[QueuedIncrement]) -> bool {
        if self.hold.load(Ordering::SeqCst) {
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
            let Some(total) = current.checked_add(operation.increment_value) else {
                return false;
            };
            working.insert(operation.key.clone(), total);
        }
        *spends = working;
        true
    }
}

#[async_trait]
impl SpendRedis for ProbeStore {
    async fn read_spends(&self, keys: &[String]) -> SpendRead {
        let spends = self.spends.lock().unwrap_or_else(PoisonError::into_inner);
        SpendRead::Values(
            keys.iter()
                .map(|key| (key.clone(), spends.get(key).copied()))
                .collect(),
        )
    }
}
