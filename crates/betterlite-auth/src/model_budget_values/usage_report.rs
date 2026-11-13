mod tests;

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;
use serde::Serialize;

use super::budget_config::{BudgetConfig, BudgetRuntime};
use super::budget_scope::BudgetScope;
use super::spend_keys::model_budget_spend_cache_key;
use super::spend_reads::error::SpendReadError;
use super::spend_reads::{SpendCache, current_window_spends};

const SPEND_DECIMAL_PLACES: u32 = 4;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ModelBudgetUsage {
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub current_spend: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub budget_limit: Option<Decimal>,
    pub time_period: String,
}

// an entry that fails validation is left out of the report without a log line
fn reportable_budgets<'budgets>(
    model_max_budget: &'budgets BTreeMap<String, JsonData>,
    runtime: BudgetRuntime<'_>,
) -> Vec<(&'budgets String, BudgetConfig)> {
    model_max_budget
        .iter()
        .filter_map(|(budget_model, raw)| {
            let budget_config = runtime.budget_config(raw).ok()?;
            Some((budget_model, budget_config))
        })
        .collect()
}

pub async fn build_model_max_budget_usage<C>(
    scope: BudgetScope,
    entity_id: Option<&str>,
    model_max_budget: Option<&BTreeMap<String, JsonData>>,
    cache: Option<&C>,
    runtime: BudgetRuntime<'_>,
) -> Result<BTreeMap<String, ModelBudgetUsage>, SpendReadError<C::Error>>
where
    C: SpendCache + Sync,
{
    let (Some(spend_cache), Some(owner), Some(configured)) = (cache, entity_id, model_max_budget)
    else {
        return Ok(BTreeMap::new());
    };
    let budgets = reportable_budgets(configured, runtime);
    if budgets.is_empty() {
        return Ok(BTreeMap::new());
    }
    let spend_keys: Vec<String> = budgets
        .iter()
        .map(|(budget_model, budget_config)| {
            model_budget_spend_cache_key(scope, owner, budget_model, &budget_config.budget_duration)
        })
        .collect();
    let current_spends = current_window_spends(spend_cache, &spend_keys).await?;
    Ok(budgets
        .into_iter()
        .zip(current_spends)
        .map(|((budget_model, budget_config), current_spend)| {
            let usage = ModelBudgetUsage {
                current_spend: current_spend.round_dp(SPEND_DECIMAL_PLACES),
                budget_limit: budget_config.max_budget,
                time_period: budget_config.budget_duration,
            };
            (budget_model.clone(), usage)
        })
        .collect())
}
