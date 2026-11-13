use std::error::Error;
use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;

use super::budget::ProviderBudget;
use super::keys::{self, WindowKeys};

// the value a window write stores: LiteLLM keeps the start time as float seconds and the spend as Decimal
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowWrite {
    Start(f64),
    Spend(Decimal),
}

// the window tier of the router cache; an unavailable store is a warning, never a failed request
#[async_trait]
pub trait BudgetWindowStore: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    async fn read_start(&self, key: &str) -> Result<Option<f64>, Self::Error>;

    async fn has_spend(&self, key: &str) -> Result<bool, Self::Error>;

    async fn write(
        &self,
        key: &str,
        value: WindowWrite,
        ttl: Option<Duration>,
    ) -> Result<(), Self::Error>;
}

// every window write lands here, so a store failure becomes one warning and never a failed request
async fn record<Store>(store: &Store, key: &str, value: WindowWrite, ttl: Option<Duration>)
where
    Store: BudgetWindowStore,
{
    if let Err(error) = store.write(key, value, ttl).await {
        tracing::warn!(%error, key, "the budget window write failed");
    }
}

// the caller's view of a window that starts now: the keys, the moment and the first cost
#[derive(Clone, Copy, Debug)]
pub struct NewWindow<'window> {
    pub keys: &'window WindowKeys,
    pub current_time: f64,
    pub response_cost: Decimal,
    pub ttl: Option<Duration>,
}

// _get_or_set_budget_start_time: a miss or an unavailable read starts the window at current_time
pub async fn get_or_set_budget_start_time<Store>(
    store: &Store,
    start_time_key: &str,
    current_time: f64,
    ttl: Option<Duration>,
) -> f64
where
    Store: BudgetWindowStore,
{
    match store.read_start(start_time_key).await {
        Ok(Some(start)) => return start,
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(%error, key = start_time_key, "the budget start time read failed; starting a new window");
        }
    }
    record(store, start_time_key, WindowWrite::Start(current_time), ttl).await;
    current_time
}

// _handle_new_budget_window: a new window stores the first cost as the spend and current_time as the start
pub async fn handle_new_budget_window<Store>(store: &Store, window: NewWindow<'_>) -> f64
where
    Store: BudgetWindowStore,
{
    record(
        store,
        &window.keys.spend,
        WindowWrite::Spend(window.response_cost),
        window.ttl,
    )
    .await;
    record(
        store,
        &window.keys.start_time,
        WindowWrite::Start(window.current_time),
        window.ttl,
    )
    .await;
    window.current_time
}

// _init_provider_budget_in_cache: start time and zero spend exist before the first request reports a cost
pub async fn init_provider_budget_in_cache<Store>(
    store: &Store,
    provider: &str,
    budget: &ProviderBudget,
    current_time: f64,
) where
    Store: BudgetWindowStore,
{
    let names = keys::provider_keys(provider, &budget.period);
    let ttl = budget.period.as_ref().map(|period| period.seconds);
    match store.read_start(&names.start_time).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            record(
                store,
                &names.start_time,
                WindowWrite::Start(current_time),
                ttl,
            )
            .await;
        }
        Err(error) => {
            tracing::warn!(%error, key = %names.start_time, "the provider budget start time read failed");
            record(
                store,
                &names.start_time,
                WindowWrite::Start(current_time),
                ttl,
            )
            .await;
        }
    }
    match store.has_spend(&names.spend).await {
        Ok(true) => {}
        Ok(false) => {
            record(store, &names.spend, WindowWrite::Spend(Decimal::ZERO), ttl).await;
        }
        Err(error) => {
            tracing::warn!(%error, key = %names.spend, "the provider budget spend read failed");
            record(store, &names.spend, WindowWrite::Spend(Decimal::ZERO), ttl).await;
        }
    }
}
