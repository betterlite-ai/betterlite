use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Serialize;

use super::error::EstimateError;
use super::ports::EstimateRouterPort;
use crate::key_fields::values::Amount;

// ProviderBudgetResponse and its per-provider object, with the amounts pydantic declares as float.
#[derive(Debug, Serialize)]
pub struct ProviderBudgetResponse {
    pub providers: IndexMap<String, ProviderBudgetResponseObject>,
}

#[derive(Debug, Serialize)]
pub struct ProviderBudgetResponseObject {
    pub budget_limit: Option<Amount>,
    pub time_period: Option<String>,
    pub spend: Amount,
    pub budget_reset_at: Option<String>,
}

// provider_budgets: every configured provider is answered from the router's budget read view, with no refresh or debit.
pub async fn provider_budgets(
    router: Option<&dyn EstimateRouterPort>,
    now: DateTime<Utc>,
) -> Result<ProviderBudgetResponse, EstimateError> {
    let Some(router) = router else {
        return Err(EstimateError::NoRouter);
    };
    let Some(config) = router.provider_budget_config() else {
        return Err(EstimateError::NoProviderBudgetConfig);
    };
    let Some(limiter) = router.budget_limiter() else {
        return Err(EstimateError::NoBudgetLimiter);
    };
    let mut providers = IndexMap::with_capacity(config.len());
    for (provider, budget) in config {
        let spend = limiter
            .current_provider_spend(provider)
            .await
            .map_err(|error| EstimateError::BudgetRead {
                message: error.to_string(),
            })?
            .unwrap_or(Decimal::ZERO);
        let reset_at = limiter
            .current_provider_budget_reset_at(provider, now)
            .await
            .map_err(|error| EstimateError::BudgetRead {
                message: error.to_string(),
            })?;
        providers.insert(
            provider.clone(),
            ProviderBudgetResponseObject {
                budget_limit: budget.max_budget.map(Amount),
                time_period: budget.budget_duration.clone(),
                spend: Amount(spend),
                budget_reset_at: reset_at,
            },
        );
    }
    Ok(ProviderBudgetResponse { providers })
}
