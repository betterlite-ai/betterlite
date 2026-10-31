use crate::budget_permission_rows::{
    JsonData, LiteLLMBudgetTable, LiteLLMBudgetTableFull, Timestamp,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLLMTeamMembership {
    pub user_id: String,
    pub team_id: String,
    #[serde(default)]
    pub budget_id: Option<String>,
    #[serde(default)]
    pub spend: Counter,
    #[serde(default)]
    pub total_spend: Counter,
    #[serde(default)]
    pub litellm_budget_table: Option<MembershipBudget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, try_from = "JsonData")]
pub enum MembershipBudget {
    Full(LiteLLMBudgetTableFull),
    Base(LiteLLMBudgetTable),
}

impl LiteLLMTeamMembership {
    #[must_use]
    pub fn safe_get_team_member_rpm_limit(&self) -> Option<i64> {
        let budget = self.litellm_budget_table.as_ref()?;
        match budget {
            MembershipBudget::Full(row) => row.budget.rpm_limit,
            MembershipBudget::Base(row) => row.rpm_limit,
        }
    }
    #[must_use]
    pub const fn safe_get_team_member_tpm_limit(&self) -> Option<i64> {
        match &self.litellm_budget_table {
            Some(MembershipBudget::Full(row)) => row.budget.tpm_limit,
            Some(MembershipBudget::Base(row)) => row.tpm_limit,
            None => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Counter(
    #[serde(serialize_with = "rust_decimal::serde::arbitrary_precision_option::serialize")]
    pub  Option<Decimal>,
);
impl Default for Counter {
    fn default() -> Self {
        Self(Some(Decimal::ZERO))
    }
}

#[derive(Deserialize)]
struct FullFields {
    created_at: Timestamp,
    budget_reset_at: Option<Timestamp>,
}
impl TryFrom<JsonData> for MembershipBudget {
    type Error = super::decoder::TeamRowError;
    fn try_from(value: JsonData) -> Result<Self, Self::Error> {
        let budget = super::decoder::convert(&value)?;
        match super::decoder::convert::<FullFields>(&value) {
            Ok(server) => Ok(Self::Full(LiteLLMBudgetTableFull {
                budget,
                created_at: server.created_at,
                budget_reset_at: server.budget_reset_at,
            })),
            Err(_) => Ok(Self::Base(budget)),
        }
    }
}
