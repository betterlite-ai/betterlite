mod tests;

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;

use super::budget_config::BudgetRuntime;
use super::candidates::BedrockCatalog;
use super::resolution::resolve_model_budget;

#[must_use]
pub fn team_model_budget_applies(
    model: &str,
    key_model_max_budget: Option<&BTreeMap<String, JsonData>>,
    catalog: &dyn BedrockCatalog,
    runtime: BudgetRuntime<'_>,
) -> bool {
    let Some(key_budgets) = key_model_max_budget.filter(|budgets| !budgets.is_empty()) else {
        return true;
    };
    resolve_model_budget(model, key_budgets, catalog, runtime).is_none_or(|resolved| {
        resolved
            .budget_config
            .max_budget
            .is_none_or(|max_budget| max_budget < Decimal::ZERO)
    })
}
