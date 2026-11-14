use std::str::FromStr as _;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_router::budget_increment::increment::BudgetIncrement;
use betterlite_router::budget_increment::window::{BudgetConfig, WindowCache as _};
use betterlite_router::budget_windows::keys::deployment_keys;
use betterlite_router::budget_windows::service::BudgetWindows;
use rust_decimal::Decimal;

use super::{ProbeCache, Step, StepKind, StepOutput};

pub(super) async fn run_step(
    step: &Step,
    windows: &BudgetWindows,
    cache: &Arc<ProbeCache>,
    initialized: bool,
) -> Result<StepOutput, String> {
    let (started, spend) = match step.kind {
        StepKind::Build => {
            return Ok(StepOutput {
                kind: StepKind::Build,
                should_init: Some(initialized),
                limiter: Some(initialized),
                tags: windows
                    .tag
                    .as_ref()
                    .map(|configs| configs.keys().cloned().collect()),
                window_started: None,
                spend: None,
            });
        }
        StepKind::Init => observe(step, cache)?,
        StepKind::Start => {
            success_event(step, windows, cache, Decimal::ZERO).await?;
            observe(step, cache)?
        }
        StepKind::Reset => reset(step, windows, cache).await?,
    };
    Ok(StepOutput {
        kind: step.kind,
        should_init: None,
        limiter: None,
        tags: None,
        window_started: Some(started),
        spend,
    })
}

// a second cost after the start moved back past the window resets the spend instead of adding to it
async fn reset(
    step: &Step,
    windows: &BudgetWindows,
    cache: &Arc<ProbeCache>,
) -> Result<(bool, Option<String>), String> {
    let start_time_key = step
        .start_time_key
        .as_deref()
        .ok_or("the reset step needs a start_time_key")?;
    let ttl = step.ttl_seconds.ok_or("the reset step needs ttl_seconds")?;
    let cost = Decimal::from_str(
        step.response_cost
            .as_deref()
            .ok_or("the reset step needs response_cost")?,
    )
    .map_err(|error| error.to_string())?;
    success_event(step, windows, cache, cost).await?;
    cache
        .write_start_time(start_time_key, 0.0, Duration::from_secs(ttl))
        .await
        .map_err(|error| error.to_string())?;
    success_event(step, windows, cache, cost).await?;
    observe(step, cache)
}

// async_log_success_event hands the deployment's budget and keys to the shared increment service
async fn success_event(
    step: &Step,
    windows: &BudgetWindows,
    cache: &Arc<ProbeCache>,
    cost: Decimal,
) -> Result<(), String> {
    let model_id = step
        .model_id
        .as_deref()
        .ok_or("the step needs a model_id")?;
    let current_time = f64::from_str(
        step.current_time
            .as_deref()
            .ok_or("the step needs current_time")?,
    )
    .map_err(|error| error.to_string())?;
    let Some(budget) = windows
        .deployment
        .as_ref()
        .and_then(|configs| configs.get(model_id))
    else {
        return Ok(());
    };
    let names = deployment_keys(model_id, &budget.period);
    let increment = BudgetIncrement {
        cache: Arc::clone(cache),
        clock: Arc::new(move || current_time),
        queue: Arc::default(),
        flush_lock: Arc::default(),
    };
    let config = BudgetConfig {
        max_budget: Some(budget.max_budget),
        budget_duration: Some(budget.period.seconds),
    };
    increment
        .increment_spend_for_key(&config, &names.spend, &names.start_time, cost)
        .await
        .map_err(|error| error.to_string())
}

fn observe(step: &Step, cache: &ProbeCache) -> Result<(bool, Option<String>), String> {
    let start_time_key = step
        .start_time_key
        .as_deref()
        .ok_or("the step needs a start_time_key")?;
    let started = cache
        .memory
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(start_time_key)
        .map_err(|error| error.to_string())?
        .is_some();
    let Some(spend_key) = step.spend_key.as_deref() else {
        return Ok((started, None));
    };
    let stored = cache
        .memory
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(spend_key)
        .map_err(|error| error.to_string())?;
    let Some(value) = stored else {
        return Ok((started, None));
    };
    let CacheValue::Object(fields) = value else {
        return Err(format!("{spend_key} does not hold a spend object"));
    };
    let Some(CacheValue::Text(text)) = fields.get("spend") else {
        return Err(format!("{spend_key} has no spend text"));
    };
    let spend = Decimal::from_str(text).map_err(|error| error.to_string())?;
    Ok((started, Some(spend.normalize().to_string())))
}
