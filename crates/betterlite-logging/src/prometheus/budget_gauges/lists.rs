use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

pub mod customer_row;
pub mod key_list;
pub mod org_list;
pub mod team_list;
pub mod user_list;

// the joined budget table a refresh row carries; both limits are absent when the row has no budget
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetTableSnapshot {
    pub max_budget: Option<Decimal>,
    pub budget_reset_at: Option<DateTime<Utc>>,
}

mod tests;
