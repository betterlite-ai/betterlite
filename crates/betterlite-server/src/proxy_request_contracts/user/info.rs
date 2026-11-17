use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::{LiteLLMObjectPermissionTable, Timestamp};
use betterlite_db::identity_rows::Payload;
use serde::{Deserialize, Serialize};

use crate::key_fields::values::Amount;
use crate::proxy_request_contracts::{
    ContractError, decode, default_amount, require_object, require_present,
};

const REQUIRED: [&str; 4] = ["user_id", "user_info", "keys", "teams"];

// LiteLLM declares every key required, and only user_id and user_info may be null.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct UserInfoResponse {
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub user_info: Option<Payload>,
    pub keys: Vec<JsonValue>,
    pub teams: Vec<JsonValue>,
}

impl UserInfoResponse {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        require_present(&fields, &REQUIRED)?;
        decode(&fields)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct UserInfoV2Response {
    pub user_id: String,
    #[serde(default)]
    pub user_email: Option<String>,
    #[serde(default)]
    pub user_alias: Option<String>,
    #[serde(default)]
    pub user_role: Option<String>,
    #[serde(default = "default_amount")]
    pub spend: Amount,
    #[serde(default)]
    pub max_budget: Option<Amount>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub budget_duration: Option<String>,
    #[serde(default)]
    pub budget_reset_at: Option<Timestamp>,
    #[serde(default)]
    pub metadata: Option<Payload>,
    #[serde(default)]
    pub created_at: Option<Timestamp>,
    #[serde(default)]
    pub updated_at: Option<Timestamp>,
    #[serde(default)]
    pub sso_user_id: Option<String>,
    #[serde(default)]
    pub teams: Vec<String>,
    #[serde(default)]
    pub object_permission: Option<LiteLLMObjectPermissionTable>,
    #[serde(default)]
    pub model_max_budget: Option<Payload>,
    #[serde(default)]
    pub model_max_budget_usage: Option<BTreeMap<String, Payload>>,
}

impl UserInfoV2Response {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        let request: Self = decode(&fields)?;
        Ok(request)
    }
}

mod tests;
