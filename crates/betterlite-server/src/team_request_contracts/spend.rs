use serde::{Deserialize, Serialize};

use crate::key_fields::values::Amount;
use crate::proxy_request_contracts::default_amount;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamUserSpendRow {
    pub team_id: String,
    pub team_alias: Option<String>,
    pub user_id: String,
    pub user_email: Option<String>,
    pub user_alias: Option<String>,
    #[serde(default = "default_amount")]
    pub spend: Amount,
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default)]
    pub api_requests: i64,
    #[serde(default)]
    pub successful_requests: i64,
    #[serde(default)]
    pub failed_requests: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamUserSpendResponse {
    pub start_date: String,
    pub end_date: String,
    pub results: Vec<TeamUserSpendRow>,
}
