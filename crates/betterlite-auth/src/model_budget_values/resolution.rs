mod tests;

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use super::budget_config::{BudgetConfig, BudgetRuntime};
use super::candidates::{BedrockCatalog, budget_model_candidates};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedModelBudget {
    pub budget_model: String,
    pub budget_config: BudgetConfig,
}

#[must_use]
pub fn resolve_model_budget(
    model: &str,
    model_max_budget: &BTreeMap<String, JsonData>,
    catalog: &dyn BedrockCatalog,
    runtime: BudgetRuntime<'_>,
) -> Option<ResolvedModelBudget> {
    for candidate in budget_model_candidates(model, catalog) {
        let raw = match model_max_budget.get(&candidate) {
            None | Some(JsonData::Null) => continue,
            Some(raw) => raw,
        };
        match runtime.budget_config(raw) {
            Ok(budget_config) => {
                return Some(ResolvedModelBudget {
                    budget_model: candidate,
                    budget_config,
                });
            }
            // a malformed entry reads as an absent one, so the next candidate gets its turn
            Err(error) => tracing::warn!(
                budget_model = %candidate,
                %error,
                "ignoring unusable model_max_budget entry; it cannot be enforced or tracked"
            ),
        }
    }
    None
}
