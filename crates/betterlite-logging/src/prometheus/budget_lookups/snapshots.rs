use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

// The auth-carried budget state a request can carry. These mirror the snapshots in betterlite-core, which depends on this crate, so the two sets of types cannot be shared; B8561 maps the carried snapshots into these shapes.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyBudgetSnapshot {
    pub budget_reset_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamBudgetSnapshot {
    pub budget_reset_at: Option<DateTime<Utc>>,
    pub max_budget: Option<Decimal>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserBudgetSnapshot {
    pub budget_reset_at: Option<DateTime<Utc>>,
    pub max_budget: Option<Decimal>,
    pub user_alias: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrgBudgetSnapshot {
    pub spend: Decimal,
    pub max_budget: Option<Decimal>,
}
