use std::sync::PoisonError;
use std::time::Duration;

use rust_decimal::Decimal;

use crate::budget_increment::dual::DualWindowCache;
use crate::budget_increment::error::WindowResult;
use crate::budget_increment::window::{WindowCache, WindowRedis};

use super::budget::ProviderBudget;
use super::keys;

// _get_or_set_budget_start_time: a miss or a failed read starts the window at current_time; unlike BudgetIncrement::window_start, a cache error only warns, as DualCache swallows it
pub async fn get_or_set_budget_start_time<Cache>(
    cache: &Cache,
    start_time_key: &str,
    current_time: f64,
    ttl: Duration,
) -> f64
where
    Cache: WindowCache,
{
    match cache.read_start_time(start_time_key).await {
        Ok(Some(start)) => return start,
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(%error, key = start_time_key, "the budget start time read failed");
        }
    }
    if let Err(error) = cache
        .write_start_time(start_time_key, current_time, ttl)
        .await
    {
        tracing::warn!(%error, key = start_time_key, "the budget start time write failed");
    }
    current_time
}

// the dual cache's get: memory first, then Redis, where spend lives as a float like the start time
pub async fn has_spend<Redis>(cache: &DualWindowCache<Redis>, key: &str) -> WindowResult<bool>
where
    Redis: WindowRedis,
{
    let cached = cache
        .memory
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)?
        .is_some();
    if cached {
        return Ok(true);
    }
    let Some(redis) = &cache.redis else {
        return Ok(false);
    };
    Ok(redis.read_start_time(key).await?.is_some())
}

// _init_provider_budget_in_cache: start time and zero spend exist before the first request reports a cost
pub async fn init_provider_budget_in_cache<Redis>(
    cache: &DualWindowCache<Redis>,
    provider: &str,
    budget: &ProviderBudget,
    current_time: f64,
) where
    Redis: WindowRedis,
{
    let Some(period) = &budget.period else {
        return;
    };
    let names = keys::provider_keys(provider, &budget.period);
    get_or_set_budget_start_time(cache, &names.start_time, current_time, period.seconds).await;
    let present = has_spend(cache, &names.spend)
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, key = %names.spend, "the provider budget spend read failed");
            false
        });
    if present {
        return;
    }
    if let Err(error) = cache
        .reset_spend(&names.spend, Decimal::ZERO, period.seconds)
        .await
    {
        tracing::warn!(%error, key = %names.spend, "the provider budget spend write failed");
    }
}
