use crate::prometheus::budget_gauges::lists::BudgetTableSnapshot;

use super::lookups::BudgetLookups;

// LiteLLM's _get_default_customer_budget: no configured id means no read, and a failed read is a debug line and None
pub async fn get_default_customer_budget(
    lookups: &dyn BudgetLookups,
    default_budget_id: Option<&str>,
) -> Option<BudgetTableSnapshot> {
    let budget_id = default_budget_id?;
    match lookups.default_customer_budget(budget_id).await {
        Ok(budget) => budget,
        Err(error) => {
            tracing::debug!(
                "[Non-Blocking] Prometheus: Error getting default customer budget: {error}"
            );
            None
        }
    }
}

mod tests;
