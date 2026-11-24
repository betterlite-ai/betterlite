use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::spend::reports::rows::{DailyTeamSpend, ScopedKeySpend, TeamModelSpend};

// get_spend_by_team_and_customer: one entry per day, each with its team and customer pairs
#[derive(Clone, Deserialize, Serialize)]
pub struct DailyTeamCustomerSpend {
    pub group_by_day: String,
    pub teams_customers: Vec<TeamCustomerSpend>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct TeamCustomerSpend {
    pub team_name: String,
    pub customer: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub total_spend: Decimal,
    pub metadata: Vec<TeamModelSpend>,
}

// the customer grouping: one entry per day, each with every end user of that day
#[derive(Clone, Deserialize, Serialize)]
pub struct DailyCustomerSpend {
    pub group_by_day: String,
    pub customers: Vec<CustomerSpend>,
}

// end_user is nullable, so a day's requests without an end user form their own null customer
#[derive(Clone, Deserialize, Serialize)]
pub struct CustomerSpend {
    pub customer: Option<String>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub total_spend: Decimal,
    pub metadata: Vec<TeamModelSpend>,
}

// the rows of whichever query the filters selected, written to the wire as they come
#[derive(Clone, Serialize)]
#[serde(untagged)]
pub enum GlobalReport {
    Keys(Vec<ScopedKeySpend>),
    TeamsCustomers(Vec<DailyTeamCustomerSpend>),
    Teams(Vec<DailyTeamSpend>),
    Customers(Vec<DailyCustomerSpend>),
}
