use super::{
    Payload, empty_payload_option, empty_vec, empty_vec_or_null, integer_option, null_decimal,
    zero_decimal,
};
use crate::budget_permission_rows::{
    JsonData, LiteLLMObjectPermissionTable, OrganizationMembership, Timestamp,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLLMUserTable {
    pub user_id: String,
    #[serde(default)]
    pub user_alias: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub sso_user_id: Option<String>,
    #[serde(default)]
    pub organization_id: Option<String>,
    #[serde(default)]
    pub object_permission_id: Option<String>,
    #[serde(default, skip_serializing)]
    pub password: Option<String>,
    #[serde(default)]
    pub password_reset_required: Option<bool>,
    #[serde(default)]
    pub last_breach_check_at: Option<Timestamp>,
    #[serde(default = "empty_vec", deserialize_with = "empty_vec_or_null")]
    pub teams: Vec<String>,
    #[serde(default)]
    pub user_role: Option<String>,
    #[serde(default)]
    pub max_budget: Option<Decimal>,
    #[serde(default = "zero_decimal", deserialize_with = "null_decimal")]
    pub spend: Decimal,
    #[serde(default)]
    pub user_email: Option<String>,
    #[serde(default = "empty_vec", deserialize_with = "empty_vec_or_null")]
    pub models: Vec<JsonData>,
    #[serde(default)]
    pub metadata: Option<Payload>,
    #[serde(default, deserialize_with = "integer_option")]
    pub max_parallel_requests: Option<i64>,
    #[serde(default, deserialize_with = "integer_option")]
    pub tpm_limit: Option<i64>,
    #[serde(default, deserialize_with = "integer_option")]
    pub rpm_limit: Option<i64>,
    #[serde(default)]
    pub budget_duration: Option<String>,
    #[serde(default)]
    pub budget_reset_at: Option<Timestamp>,
    #[serde(default)]
    pub allowed_cache_controls: Vec<String>,
    #[serde(default)]
    pub policies: Vec<String>,
    #[serde(default = "empty_payload_option")]
    pub model_spend: Option<Payload>,
    #[serde(default = "empty_payload_option")]
    pub model_max_budget: Option<Payload>,
    #[serde(default)]
    pub created_at: Option<Timestamp>,
    #[serde(default)]
    pub updated_at: Option<Timestamp>,
    #[serde(default)]
    pub organization_memberships: Option<Vec<OrganizationMembership>>,
    #[serde(default)]
    pub object_permission: Option<LiteLLMObjectPermissionTable>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScimPlaceholder {
    pub placeholder_user_id: String,
    pub resolved_user_ids: Vec<String>,
    pub team_ids: Vec<String>,
}
