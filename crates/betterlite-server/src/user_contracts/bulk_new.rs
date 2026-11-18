use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use serde::{Deserialize, Serialize};

use crate::proxy_request_contracts::user::NewUserRequest;
use crate::proxy_request_contracts::{ContractError, missing_field, require_object};
use crate::user_contracts::wire::{check_count, reject_unknown_fields, wrong_type};

pub const MAX_BULK_NEW_USERS: usize = 500;

const INVITE_UNSUPPORTED: &str =
    "send_invite_email is not supported on /management/v1/users/bulk; invite users separately";
const NEW_USER_ITEM_FIELDS: [&str; 38] = [
    "agent_id",
    "aliases",
    "allowed_cache_controls",
    "auto_create_key",
    "blocked",
    "budget_duration",
    "budget_fallbacks",
    "budget_limits",
    "config",
    "duration",
    "guardrails",
    "key_alias",
    "max_budget",
    "max_parallel_requests",
    "mcp_rpm_limit",
    "metadata",
    "model_max_budget",
    "model_rpm_limit",
    "model_tpm_limit",
    "models",
    "object_permission",
    "organizations",
    "password",
    "permissions",
    "policies",
    "prompts",
    "rpm_limit",
    "send_invite_email",
    "sso_user_id",
    "spend",
    "tag_rpm_limit",
    "team_id",
    "teams",
    "tpm_limit",
    "user_alias",
    "user_email",
    "user_id",
    "user_role",
];

#[derive(Debug, Clone)]
pub struct BulkNewUserItem {
    pub request: NewUserRequest,
}

impl BulkNewUserItem {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let mut fields = require_object(value)?;
        reject_unknown_fields(&fields, &NEW_USER_ITEM_FIELDS)?;
        fields
            .entry("auto_create_key".to_owned())
            .or_insert(JsonData::Bool(false));
        let document: JsonValue = serde_json::to_value(JsonData::Object(fields))?;
        let request = NewUserRequest::from_value(&document)?;
        if request.send_invite_email == Some(true) {
            return Err(ContractError::from(INVITE_UNSUPPORTED));
        }
        Ok(Self { request })
    }
}

#[derive(Debug, Clone)]
pub struct BulkNewUserRequest {
    pub users: Vec<BulkNewUserItem>,
}

impl BulkNewUserRequest {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        reject_unknown_fields(&fields, &["users"])?;
        let Some(found) = fields.get("users") else {
            return Err(missing_field("users"));
        };
        let JsonData::Array(items) = found else {
            return Err(wrong_type("users"));
        };
        let mut users = Vec::with_capacity(items.len());
        for item in items {
            let document: JsonValue = serde_json::to_value(item)?;
            users.push(BulkNewUserItem::from_value(&document)?);
        }
        check_count("users", users.len(), MAX_BULK_NEW_USERS)?;
        Ok(Self { users })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserCreateResult {
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub user_email: Option<String>,
    pub success: bool,
    #[serde(default)]
    pub teams: Option<Vec<String>>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkNewUserMeta {
    pub total_requested: i64,
    pub created: i64,
    pub failed: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkNewUserResponse {
    pub data: Vec<UserCreateResult>,
    pub meta: BulkNewUserMeta,
}
