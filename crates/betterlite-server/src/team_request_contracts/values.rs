use betterlite_auth::management_routes::key_routes::KEY_ENUM;
use serde::{Deserialize, Serialize};

use crate::key_fields::values::Amount;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TeamIdSearchMatch {
    Exact,
    Prefix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThroughputLimitType {
    #[serde(rename = "guaranteed_throughput")]
    Guaranteed,
    #[serde(rename = "best_effort_throughput")]
    BestEffort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetSource {
    Member,
    TeamDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "i64")]
pub struct PositiveInt(pub i64);

impl TryFrom<i64> for PositiveInt {
    type Error = NotPositiveError;
    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if value > 0 {
            return Ok(Self(value));
        }
        Err(NotPositiveError)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Input should be a valid integer greater than 0")]
pub struct NotPositiveError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct KeyManagementRoutes(String);

impl TryFrom<String> for KeyManagementRoutes {
    type Error = UnknownRouteError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let found = KEY_ENUM.iter().find(|route| route.value() == value);
        match found {
            Some(route) => Ok(Self(route.value().to_owned())),
            None => Err(UnknownRouteError { value }),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Input should be a valid key management route, not '{value}'")]
pub struct UnknownRouteError {
    value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "BudgetConfigInput")]
pub struct BudgetConfig {
    pub max_budget: Option<Amount>,
    pub budget_duration: Option<String>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
}

#[derive(Deserialize)]
struct BudgetConfigInput {
    max_budget: Option<Amount>,
    budget_duration: Option<String>,
    tpm_limit: Option<i64>,
    rpm_limit: Option<i64>,
    budget_limit: Option<Amount>,
    time_period: Option<String>,
}

impl From<BudgetConfigInput> for BudgetConfig {
    fn from(input: BudgetConfigInput) -> Self {
        Self {
            max_budget: input.budget_limit.or(input.max_budget),
            budget_duration: input.time_period.or(input.budget_duration),
            tpm_limit: input.tpm_limit,
            rpm_limit: input.rpm_limit,
        }
    }
}
