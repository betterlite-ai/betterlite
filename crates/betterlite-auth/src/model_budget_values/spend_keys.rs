mod tests;

use super::budget_scope::BudgetScope;

#[must_use]
pub fn model_budget_spend_cache_key(
    scope: BudgetScope,
    entity_id: &str,
    budget_model: &str,
    budget_duration: &str,
) -> String {
    let prefixes = scope.prefixes();
    format!(
        "{}:{entity_id}:{budget_model}:{budget_duration}",
        prefixes.spend
    )
}
