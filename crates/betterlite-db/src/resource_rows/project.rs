use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::budget_permission_rows::{LiteLLMBudgetTable, LiteLLMObjectPermissionTable, Timestamp};
use crate::identity_rows::Payload;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectTable {
    pub project_id: String,
    #[serde(default)]
    pub project_alias: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub budget_id: Option<String>,
    #[serde(default)]
    pub metadata: Option<Payload>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision")]
    pub spend: Decimal,
    #[serde(default)]
    pub model_spend: Option<Payload>,
    #[serde(default)]
    pub model_rpm_limit: Option<Payload>,
    #[serde(default)]
    pub model_tpm_limit: Option<Payload>,
    #[serde(default)]
    pub blocked: bool,
    #[serde(default)]
    pub object_permission_id: Option<String>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub updated_by: Option<String>,
    #[serde(default)]
    pub created_at: Option<Timestamp>,
    #[serde(default)]
    pub updated_at: Option<Timestamp>,
    #[serde(default)]
    pub litellm_budget_table: Option<LiteLLMBudgetTable>,
    #[serde(default)]
    pub object_permission: Option<LiteLLMObjectPermissionTable>,
}
