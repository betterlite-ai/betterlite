use rust_decimal::Decimal;
use std::collections::BTreeMap;

use crate::budget_permission_rows::LiteLLMBudgetTableFull;
use crate::budget_reset_policy::rollover::RolloverPolicy;

// _rollover_cap per expiring tier, and nothing at all when litellm.budget_rollover is off: a tier missing from this map is zeroed, not carried.
#[must_use]
pub fn rollover_caps(
    budgets: &[LiteLLMBudgetTableFull],
    policy: RolloverPolicy,
) -> BTreeMap<String, Decimal> {
    if !policy.enabled {
        return BTreeMap::new();
    }
    budgets
        .iter()
        .filter_map(|budget| {
            let budget_id = budget.budget.budget_id.as_ref()?;
            let cap = budget.budget.max_budget?;
            Some((budget_id.clone(), cap))
        })
        .collect()
}

mod tests;
