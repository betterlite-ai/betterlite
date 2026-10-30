use super::aliases::LiteLLMModelTable;
use super::member::{BudgetLimitEntry, Member};
use crate::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable, Timestamp};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamBase<Id = Option<String>> {
    #[serde(default)]
    pub team_alias: Option<String>,
    #[serde(default)]
    pub team_id: Id,
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
    pub metadata: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub tpm_limit: Option<i64>,
    #[serde(default)]
    pub rpm_limit: Option<i64>,
    #[serde(default)]
    pub tpd_limit: Option<i64>,
    #[serde(
        default,
        serialize_with = "rust_decimal::serde::arbitrary_precision_option::serialize"
    )]
    pub max_budget: Option<Decimal>,
    #[serde(
        default,
        serialize_with = "rust_decimal::serde::arbitrary_precision_option::serialize"
    )]
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
    pub router_settings: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub access_group_ids: Option<Vec<String>>,
    #[serde(default)]
    pub default_team_member_models: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamFields {
    #[serde(flatten)]
    pub base: TeamBase<String>,
    #[serde(
        default,
        serialize_with = "rust_decimal::serde::arbitrary_precision_option::serialize"
    )]
    pub spend: Option<Decimal>,
    #[serde(default)]
    pub max_parallel_requests: Option<i64>,
    #[serde(default)]
    pub budget_reset_at: Option<Timestamp>,
    #[serde(default)]
    pub model_id: Option<i64>,
    #[serde(default)]
    pub model_spend: Nullable<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub model_max_budget: Nullable<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub policies: Option<Vec<String>>,
    #[serde(default)]
    pub allow_team_guardrail_config: Nullable<bool>,
    #[serde(default)]
    pub litellm_model_table: Option<LiteLLMModelTable>,
    #[serde(default)]
    pub object_permission: Option<LiteLLMObjectPermissionTable>,
    #[serde(default)]
    pub object_permission_id: Option<String>,
    #[serde(default)]
    pub updated_at: Option<Timestamp>,
    #[serde(default)]
    pub created_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "JsonData")]
pub struct LiteLLMTeamTable {
    #[serde(flatten)]
    pub fields: TeamFields,
}
impl TryFrom<JsonData> for LiteLLMTeamTable {
    type Error = super::decoder::TeamRowError;
    fn try_from(mut value: JsonData) -> Result<Self, Self::Error> {
        super::validator::set_model_info(&mut value)?;
        let JsonData::Object(fields) = &value else {
            return Err(Self::Error::NotAnObject);
        };
        if !fields.contains_key("team_id") {
            return Err(Self::Error::MissingTeamId);
        }
        Ok(Self {
            fields: super::decoder::convert(&value)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Nullable<T>(pub Option<T>);
impl<T> Default for Nullable<T>
where
    T: Default,
{
    fn default() -> Self {
        Self(Some(T::default()))
    }
}
