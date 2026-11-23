use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_user_lookup::ports::{BudgetResetCalculator, PortError};
use betterlite_db::budget_permission_rows::Timestamp;
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;
use tracing::warn;

use crate::startup_checks::error::StartupCheckError;
use crate::startup_checks::presence::DatabasePresence;

// litellm/constants.py LITELLM_PROXY_BUDGET_NAME: the user row a proxy-wide max_budget is stored on
pub const LITELLM_PROXY_BUDGET_NAME: &str = "litellm-proxy-budget";

#[async_trait]
pub trait ProxyBudgetStore: Send + Sync {
    async fn upsert_proxy_budget(
        &self,
        user_id: &str,
        max_budget: Decimal,
        budget_duration: &str,
    ) -> Result<(), PortError>;
    // update_many where user_id matches and budget_reset_at is null, setting budget_reset_at and spend 0
    async fn backfill_null_budget_reset(
        &self,
        user_id: &str,
        reset_at: Timestamp,
    ) -> Result<(), PortError>;
}

pub struct ProxyBudget<'input> {
    pub max_budget: Decimal,
    pub budget_duration: Option<&'input str>,
    pub database: DatabasePresence,
}

pub fn seed_proxy_budget(
    budget: &ProxyBudget<'_>,
    store: Arc<dyn ProxyBudgetStore>,
    reset_calculator: Arc<dyn BudgetResetCalculator>,
    tasks: &TaskTracker,
) -> Result<(), StartupCheckError> {
    if budget.database == DatabasePresence::Absent || budget.max_budget <= Decimal::ZERO {
        return Ok(());
    }
    let Some(budget_duration) = budget.budget_duration.map(str::to_owned) else {
        return Err(StartupCheckError::BudgetDurationNotSet);
    };
    let max_budget = budget.max_budget;
    tasks.spawn(async move {
        write_proxy_budget(
            store.as_ref(),
            reset_calculator.as_ref(),
            max_budget,
            &budget_duration,
        )
        .await;
    });
    Ok(())
}

async fn write_proxy_budget(
    store: &dyn ProxyBudgetStore,
    reset_calculator: &dyn BudgetResetCalculator,
    max_budget: Decimal,
    budget_duration: &str,
) {
    if let Err(error) = store
        .upsert_proxy_budget(LITELLM_PROXY_BUDGET_NAME, max_budget, budget_duration)
        .await
    {
        warn!("Failed to upsert the proxy budget row: {error}");
        return;
    }
    let reset_at = match reset_calculator.reset_time(budget_duration) {
        Ok(reset_at) => reset_at,
        Err(error) => {
            warn!("Failed to backfill budget_reset_at on proxy admin row: {error}");
            return;
        }
    };
    if let Err(error) = store
        .backfill_null_budget_reset(LITELLM_PROXY_BUDGET_NAME, reset_at)
        .await
    {
        warn!("Failed to backfill budget_reset_at on proxy admin row: {error}");
    }
}
