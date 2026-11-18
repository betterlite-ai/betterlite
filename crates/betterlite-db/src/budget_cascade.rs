pub mod cache_invalidation;
pub mod collect_cascade;
pub mod enduser_counter_key;
pub mod enduser_invalidation;
pub mod fetch_failure;
pub mod key__counter_key;
pub mod linked_rows;
pub mod model_access_group_counter_key;
pub mod org__counter_key;
pub mod project_counter_key;
pub mod rollover_carry;
pub mod rollover_order;
pub mod tag__counter_key;
pub mod team_membership_counter_key;
pub mod transaction;

mod tests;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::collections::BTreeMap;

use crate::budget_permission_rows::LiteLLMBudgetTableFull;

// Everything one budget-tier reset touches, resolved before any write; the phase reads it after the commit for end-user invalidation and progress metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BudgetCascade {
    pub budgets: Vec<LiteLLMBudgetTableFull>,
    pub budget_ids: Vec<String>,
    pub budget_resets: Vec<BudgetReset>,
    pub counter_resets: Vec<CounterReset>,
    pub cache_keys: Vec<String>,
    pub rollover_caps: BTreeMap<String, Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetReset {
    pub budget_id: String,
    pub reset_at: DateTime<Utc>,
}

// The carry-over value rides along with its counter key even though invalidation only needs the key, so the record matches the source's counter_resets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CounterReset {
    pub counter_key: String,
    pub carried_spend: Decimal,
}
