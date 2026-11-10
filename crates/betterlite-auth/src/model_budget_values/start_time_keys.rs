mod tests;

use super::budget_scope::BudgetScope;

#[must_use]
// ast-grep-ignore: no-long-names
pub fn model_budget_start_time_cache_key(
    scope: BudgetScope,
    entity_id: &str,
    budget_model: &str,
    budget_duration: &str,
) -> String {
    let prefixes = scope.prefixes();
    format!(
        "{}:{entity_id}:{budget_model}:{budget_duration}",
        prefixes.start_time
    )
}
