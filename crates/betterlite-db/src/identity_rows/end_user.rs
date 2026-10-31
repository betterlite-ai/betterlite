use super::{PydanticBool, null_decimal};
use crate::budget_permission_rows::{LiteLLMBudgetTable, LiteLLMObjectPermissionTable};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLLMEndUserTable {
    pub user_id: String,
    pub blocked: PydanticBool,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(
        default,
        serialize_with = "rust_decimal::serde::float::serialize",
        deserialize_with = "null_decimal"
    )]
    pub spend: Decimal,
    #[serde(default)]
    pub allowed_model_region: Option<Region>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub budget_id: Option<String>,
    #[serde(default)]
    pub litellm_budget_table: Option<LiteLLMBudgetTable>,
    #[serde(default)]
    pub object_permission_id: Option<String>,
    #[serde(default)]
    pub object_permission: Option<LiteLLMObjectPermissionTable>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Region {
    Eu,
    Us,
}
