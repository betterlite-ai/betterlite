#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::{CacheValue, LocalTtlCache};
use betterlite_router::budget_increment::error::IncrementError;
use betterlite_router::budget_increment::queue::QueuedIncrement;
use betterlite_router::budget_increment::window::WindowRedis;
use betterlite_router::budget_windows::budget::{BudgetPeriod, ProviderBudget};
use betterlite_router::budget_windows::service::BudgetWindows;
use betterlite_router::routing_budget_state::event::SuccessEvent;
use betterlite_router::routing_budget_state::spend_redis::{SpendRead, SpendRedis};
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::routing_budget_state;

const SPEND_KEY: &str = "provider_spend:openai:1d";

#[derive(Clone, Default)]
struct RecordingStore {
    spends: Arc<Mutex<BTreeMap<String, Decimal>>>,
}

#[async_trait]
impl WindowRedis for RecordingStore {
    async fn read_start_time(&self, _key: &str) -> Result<Option<f64>, IncrementError> {
        Ok(None)
    }

    async fn write_start_time(&self, _key: &str, _start_time: f64, _ttl: Duration) {}

    async fn write_spend(&self, _key: &str, _cost: Decimal, _ttl: Duration) {}

    async fn write_increments(&self, operations: &[QueuedIncrement]) -> bool {
        let mut spends = self.spends.lock().unwrap();
        for operation in operations {
            let total = spends.entry(operation.key.clone()).or_default();
            *total = total.saturating_add(operation.increment_value);
        }
        true
    }
}

#[async_trait]
impl SpendRedis for RecordingStore {
    async fn read_spends(&self, keys: &[String]) -> SpendRead {
        let spends = self.spends.lock().unwrap();
        SpendRead::Values(
            keys.iter()
                .map(|key| (key.clone(), spends.get(key).copied()))
                .collect(),
        )
    }
}

fn windows() -> BudgetWindows {
    let period = BudgetPeriod {
        spec: "1d".to_owned(),
        seconds: Duration::from_secs(86_400),
    };
    let providers = IndexMap::from([(
        "openai".to_owned(),
        ProviderBudget {
            budget_limit: Some(Decimal::from(10_u8)),
            period: Some(period),
        },
    )]);
    BudgetWindows::new(Some(providers), None, &[])
}

fn clock() -> Arc<dyn Fn() -> f64 + Send + Sync> {
    Arc::new(|| 0.0_f64)
}

#[tokio::test]
async fn factory_assembles_a_state_that_spends() {
    let memory = Arc::new(Mutex::new(
        LocalTtlCache::with_clock(None, None, None, 1024, clock()).unwrap(),
    ));
    let store = RecordingStore::default();
    let state = routing_budget_state(windows(), Arc::clone(&memory), store.clone(), clock());
    let event = SuccessEvent {
        call_type: None,
        response_cost: Decimal::from_str("0.5").unwrap(),
        model_id: None,
        custom_llm_provider: Some("openai"),
        tags: &[],
    };
    state.log_success(&event).await.unwrap();
    assert!(state.flush().await);
    let recorded = store.spends.lock().unwrap();
    assert_eq!(
        recorded.get(SPEND_KEY),
        Some(&Decimal::from_str("0.5").unwrap())
    );
    let local = memory.lock().unwrap().get(SPEND_KEY).unwrap();
    assert!(matches!(local, Some(CacheValue::Object(_))));
}
