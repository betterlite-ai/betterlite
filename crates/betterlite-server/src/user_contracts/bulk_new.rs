use betterlite_auth::user_roles::UserRole;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::key_fields::base::GenerateRequestBase;
use crate::key_fields::decode::blank_to_null;
use crate::key_fields::values::Amount;
use crate::proxy_request_contracts::user::{NewUserRequest, NewUserTeams};
use crate::proxy_request_contracts::{ContractError, Fields, decode, require_object};
use crate::user_contracts::wire::{
    PASSWORD_UNSUPPORTED, check_count, check_user_role, reject_unknown_fields,
};

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

// One /user/new body: B8411's fields, with auto_create_key off unless the row opts in.
#[derive(Deserialize)]
struct BulkNewUserItemWire {
    #[serde(flatten)]
    base: GenerateRequestBase,
    #[serde(default)]
    max_budget: Option<Amount>,
    #[serde(default)]
    user_email: Option<String>,
    #[serde(default)]
    user_alias: Option<String>,
    #[serde(default)]
    user_role: Option<UserRole>,
    #[serde(default)]
    teams: Option<NewUserTeams>,
    #[serde(default)]
    auto_create_key: bool,
    #[serde(default)]
    send_invite_email: Option<bool>,
    #[serde(default)]
    sso_user_id: Option<String>,
    #[serde(default)]
    organizations: Option<Vec<String>>,
    #[serde(default)]
    password: Option<SecretString>,
}

// Every /management/v1 body forbids extras, and the inherited max_budget validator turns a blank into null.
fn prepare_rows(fields: &mut Fields) -> Result<(), ContractError> {
    let Some(JsonData::Array(rows)) = fields.get_mut("users") else {
        return Ok(());
    };
    for row in rows {
        if let JsonData::Object(row_fields) = row {
            reject_unknown_fields(row_fields, &NEW_USER_ITEM_FIELDS)?;
            blank_to_null(row_fields, &["max_budget"]);
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct BulkNewUserItem {
    pub request: NewUserRequest,
}

impl BulkNewUserItem {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let mut fields = require_object(value)?;
        reject_unknown_fields(&fields, &NEW_USER_ITEM_FIELDS)?;
        blank_to_null(&mut fields, &["max_budget"]);
        let wire: BulkNewUserItemWire = decode(&fields)?;
        Self::from_wire(wire)
    }

    fn from_wire(wire: BulkNewUserItemWire) -> Result<Self, ContractError> {
        // the inherited validators run in LiteLLM's field order: role, invite flag, password
        check_user_role(wire.user_role)?;
        if wire.send_invite_email == Some(true) {
            return Err(ContractError::from(INVITE_UNSUPPORTED));
        }
        if wire.password.is_some() {
            return Err(ContractError::from(PASSWORD_UNSUPPORTED));
        }
        let request = NewUserRequest {
            base: wire.base,
            max_budget: wire.max_budget,
            user_email: wire.user_email,
            user_alias: wire.user_alias,
            user_role: wire.user_role,
            teams: wire.teams,
            auto_create_key: wire.auto_create_key,
            send_invite_email: wire.send_invite_email,
            sso_user_id: wire.sso_user_id,
            organizations: wire.organizations,
            password: wire.password,
        };
        Ok(Self { request })
    }
}

// The wrapper is a typed wire record; each row keeps the /user/new model's fields.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BulkNewUserWire {
    users: Vec<BulkNewUserItemWire>,
}

#[derive(Debug, Clone)]
pub struct BulkNewUserRequest {
    pub users: Vec<BulkNewUserItem>,
}

impl BulkNewUserRequest {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let mut fields = require_object(value)?;
        prepare_rows(&mut fields)?;
        let wire: BulkNewUserWire = decode(&fields)?;
        let mut users = Vec::with_capacity(wire.users.len());
        for row in wire.users {
            users.push(BulkNewUserItem::from_wire(row)?);
        }
        check_count("users", users.len(), MAX_BULK_NEW_USERS)?;
        Ok(Self { users })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserCreateResult {
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub user_email: Option<String>,
    pub success: bool,
    #[serde(default)]
    pub teams: Option<Vec<String>>,
    #[serde(
        default,
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<crate::key_fields::values::ExposedSecret>>>::serialize"
    )]
    pub key: Option<SecretString>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkNewUserMeta {
    pub total_requested: i64,
    pub created: i64,
    pub failed: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkNewUserResponse {
    pub data: Vec<UserCreateResult>,
    pub meta: BulkNewUserMeta,
}
