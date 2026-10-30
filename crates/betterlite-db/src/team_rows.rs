use chrono::{DateTime, FixedOffset, NaiveDateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::budget_permission_rows::{
    LiteLLMBudgetTable, LiteLLMBudgetTableFull, LiteLLMObjectPermissionTable,
};

mod decoder;
pub use decoder::{TeamRowError, decode_team_rows};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TeamTimestamp {
    Aware(DateTime<FixedOffset>),
    Naive(NaiveDateTime),
}

impl TeamTimestamp {
    #[must_use]
    pub fn utc(self) -> DateTime<Utc> {
        match self {
            Self::Aware(value) => value.with_timezone(&Utc),
            Self::Naive(value) => value.and_utc(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberBase {
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub user_email: Option<String>,
}

impl MemberBase {
    pub fn validate(value: &Value) -> Result<(), TeamRowError> {
        let object = value
            .as_object()
            .ok_or_else(|| TeamRowError::Validation("input needs to be a dictionary".to_owned()))?;
        if object.get("user_id").is_none_or(Value::is_null)
            && object.get("user_email").is_none_or(Value::is_null)
        {
            return Err(TeamRowError::Validation(
                "Either user id or user email must be provided".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    #[serde(flatten)]
    pub base: MemberBase,
    pub role: MemberRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemberRole {
    Admin,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetLimitEntry {
    pub budget_duration: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub max_budget: Decimal,
    #[serde(default)]
    pub reset_at: Option<TeamTimestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLLM_ModelTable {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub model_aliases: Option<ModelAliases>,
    pub created_by: String,
    pub updated_by: String,
    #[serde(default)]
    pub team: Option<Box<LiteLLM_TeamTable>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModelAliases {
    Text(String),
    Map(serde_json::Map<String, Value>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamBase {
    #[serde(default)]
    pub team_alias: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub organization_id: Option<String>,
    #[serde(default)]
    pub admins: Vec<String>,
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default)]
    pub members_with_roles: Vec<Member>,
    #[serde(default)]
    pub team_member_permissions: Option<Vec<String>>,
    #[serde(default)]
    pub metadata: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    pub tpm_limit: Option<i64>,
    #[serde(default)]
    pub rpm_limit: Option<i64>,
    #[serde(default)]
    pub tpd_limit: Option<i64>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub max_budget: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub soft_budget: Option<Decimal>,
    #[serde(default)]
    pub budget_duration: Option<String>,
    #[serde(default)]
    pub budget_limits: Option<Vec<BudgetLimitEntry>>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub blocked: bool,
    #[serde(default)]
    pub router_settings: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    pub access_group_ids: Option<Vec<String>>,
    #[serde(default)]
    pub default_team_member_models: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLLM_TeamTable {
    #[serde(flatten)]
    pub base: TeamBase,
    pub team_id: String,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub spend: Option<Decimal>,
    #[serde(default)]
    pub max_parallel_requests: Option<i64>,
    #[serde(default)]
    pub budget_reset_at: Option<TeamTimestamp>,
    #[serde(default)]
    pub model_id: Option<i64>,
    #[serde(default)]
    pub model_spend: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    pub model_max_budget: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    pub policies: Option<Vec<String>>,
    #[serde(default)]
    pub allow_team_guardrail_config: Option<bool>,
    #[serde(default)]
    pub litellm_model_table: Option<LiteLLM_ModelTable>,
    #[serde(default)]
    pub object_permission: Option<LiteLLMObjectPermissionTable>,
    #[serde(default)]
    pub object_permission_id: Option<String>,
    #[serde(default)]
    pub updated_at: Option<TeamTimestamp>,
    #[serde(default)]
    pub created_at: Option<TeamTimestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLLM_TeamTableCachedObj {
    #[serde(flatten)]
    pub team: LiteLLM_TeamTable,
    #[serde(default)]
    pub last_refreshed_at: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLLM_DeletedTeamTable {
    #[serde(flatten)]
    pub team: LiteLLM_TeamTable,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub deleted_at: Option<TeamTimestamp>,
    #[serde(default)]
    pub deleted_by: Option<String>,
    #[serde(default)]
    pub deleted_by_api_key: Option<String>,
    #[serde(default)]
    pub litellm_changed_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLLM_TeamMembership {
    pub user_id: String,
    pub team_id: String,
    #[serde(default)]
    pub budget_id: Option<String>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub spend: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub total_spend: Option<Decimal>,
    #[serde(default)]
    pub litellm_budget_table: Option<MembershipBudget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MembershipBudget {
    Full(LiteLLMBudgetTableFull),
    Base(LiteLLMBudgetTable),
}

impl LiteLLM_TeamMembership {
    #[must_use]
    pub fn safe_get_team_member_rpm_limit(&self) -> Option<i64> {
        let limit = self
            .litellm_budget_table
            .as_ref()
            .and_then(|budget| match budget {
                MembershipBudget::Full(row) => row.budget.rpm_limit,
                MembershipBudget::Base(row) => row.rpm_limit,
            });
        limit
    }
    #[must_use]
    pub fn safe_get_team_member_tpm_limit(&self) -> Option<i64> {
        let limit = self
            .litellm_budget_table
            .as_ref()
            .and_then(|budget| match budget {
                MembershipBudget::Full(row) => row.budget.tpm_limit,
                MembershipBudget::Base(row) => row.tpm_limit,
            });
        limit
    }
}

pub fn set_model_info(value: &mut Value) -> Result<(), TeamRowError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| TeamRowError::Validation("input needs to be a dictionary".to_owned()))?;
    if object
        .get("members_with_roles")
        .is_some_and(|item| item.as_object().is_some_and(serde_json::Map::is_empty))
    {
        object.insert("members_with_roles".to_owned(), Value::Array(Vec::new()));
    }
    for field in [
        "metadata",
        "aliases",
        "config",
        "permissions",
        "model_max_budget",
        "model_aliases",
        "router_settings",
        "budget_limits",
    ] {
        let Some(item) = object.get_mut(field) else {
            continue;
        };
        if let Value::String(text) = item {
            *item =
                serde_json::from_str(text).map_err(|_| TeamRowError::Field(field.to_owned()))?;
        }
    }
    Ok(())
}

#[path = "team_rows/tests.rs"]
mod tests;
