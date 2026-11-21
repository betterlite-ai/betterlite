use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamBudgetGaugeInput {
    pub team_id: String,
    pub team_alias: Option<String>,
    pub spend: Option<Decimal>,
    pub max_budget: Option<Decimal>,
    pub budget_reset_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrgBudgetGaugeInput {
    pub org_id: String,
    pub org_alias: String,
    pub spend: Decimal,
    pub max_budget: Option<Decimal>,
    pub budget_reset_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyBudgetGaugeInput {
    pub token: String,
    pub key_alias: Option<String>,
    pub spend: Option<Decimal>,
    pub max_budget: Option<Decimal>,
    pub budget_reset_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserBudgetGaugeInput {
    pub user_id: String,
    pub user_email: Option<String>,
    pub user_alias: Option<String>,
    pub spend: Decimal,
    pub max_budget: Option<Decimal>,
    pub budget_reset_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CustomerBudgetGaugeInput {
    pub end_user_id: String,
    pub spend: Decimal,
    pub max_budget: Option<Decimal>,
    pub budget_reset_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeamMembersGaugeInput {
    pub team_id: String,
    pub team_alias: Option<String>,
    pub member_count: usize,
}
