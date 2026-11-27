use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::prometheus::budget_gauges::lists::BudgetTableSnapshot;

// The columns the budget lookups read, one record per LiteLLM model the lookup returns.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamRecord {
    pub budget_reset_at: Option<DateTime<Utc>>,
    pub max_budget: Option<Decimal>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrgRecord {
    pub spend: Option<Decimal>,
    pub organization_alias: Option<String>,
    pub litellm_budget_table: Option<BudgetTableSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyRecord {
    pub budget_reset_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserRecord {
    pub budget_reset_at: Option<DateTime<Utc>>,
    pub max_budget: Option<Decimal>,
    pub user_email: Option<String>,
    pub user_alias: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedEndUser {
    pub spend: Decimal,
    pub litellm_budget_table: Option<BudgetTableSnapshot>,
}
