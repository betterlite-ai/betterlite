use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::budget_permission_rows::{LiteLLMBudgetTable, LiteLLMObjectPermissionTable};
use crate::identity_rows::user::LiteLLMUserTable;
use crate::identity_rows::{OptionalPayload, Payload};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrganizationTable {
    #[serde(default)]
    pub organization_id: Option<String>,
    #[serde(default)]
    pub organization_alias: Option<String>,
    pub budget_id: String,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision")]
    pub spend: Decimal,
    #[serde(default)]
    pub metadata: Option<Payload>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub model_spend: OptionalPayload,
    pub created_by: String,
    pub updated_by: String,
    #[serde(default)]
    pub users: Option<Vec<LiteLLMUserTable>>,
    #[serde(default)]
    pub litellm_budget_table: Option<LiteLLMBudgetTable>,
    #[serde(default)]
    pub object_permission: Option<LiteLLMObjectPermissionTable>,
    #[serde(default)]
    pub object_permission_id: Option<String>,
}
