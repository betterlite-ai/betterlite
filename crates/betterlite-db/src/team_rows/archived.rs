use super::team::LiteLLMTeamTable;
use crate::budget_permission_rows::Timestamp;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLLMTeamTableCachedObj {
    #[serde(flatten)]
    pub team: LiteLLMTeamTable,
    #[serde(
        default,
        serialize_with = "rust_decimal::serde::arbitrary_precision_option::serialize"
    )]
    pub last_refreshed_at: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLLMDeletedTeamTable {
    #[serde(flatten)]
    pub team: LiteLLMTeamTable,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub deleted_at: Option<Timestamp>,
    #[serde(default)]
    pub deleted_by: Option<String>,
    #[serde(default)]
    pub deleted_by_api_key: Option<String>,
    #[serde(default)]
    pub litellm_changed_by: Option<String>,
}
