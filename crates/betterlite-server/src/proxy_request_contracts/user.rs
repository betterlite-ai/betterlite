use std::collections::BTreeSet;

use betterlite_auth::user_roles::UserRole;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::identity_rows::Payload;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use super::{ContractError, Fields, decode, is_set, require_object, supplied_fields};
use crate::key_fields::base::GenerateRequestBase;
use crate::key_fields::decode::blank_to_null;
use crate::key_fields::values::Amount;

const PASSWORD_UNSUPPORTED: &str = "password cannot be set via /user/new. Users set their own password through an invitation link (POST /invitation/new).";
const USER_SELECTOR: &str = "Either user id or user email must be provided";

// The six fields UpdateUserRequestNoUserIDorEmail adds to GenerateRequestBase.
pub(crate) const USER_FIELDS: [&str; 6] = [
    "password",
    "spend",
    "metadata",
    "user_alias",
    "user_role",
    "max_budget",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamUserRole {
    #[default]
    User,
    Admin,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct NewUserRequestTeam {
    pub team_id: String,
    #[serde(default)]
    pub max_budget_in_team: Option<Amount>,
    #[serde(default)]
    pub user_role: TeamUserRole,
}

impl NewUserRequestTeam {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        let request: Self = decode(&fields)?;
        Ok(request)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum NewUserTeams {
    Ids(Vec<String>),
    Records(Vec<NewUserRequestTeam>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewUserRequest {
    #[serde(flatten)]
    pub base: GenerateRequestBase,
    #[serde(default)]
    pub max_budget: Option<Amount>,
    #[serde(default)]
    pub user_email: Option<String>,
    #[serde(default)]
    pub user_alias: Option<String>,
    #[serde(default)]
    pub user_role: Option<UserRole>,
    #[serde(default)]
    pub teams: Option<NewUserTeams>,
    #[serde(default = "default_auto_create_key")]
    pub auto_create_key: bool,
    #[serde(default)]
    pub send_invite_email: Option<bool>,
    #[serde(default)]
    pub sso_user_id: Option<String>,
    #[serde(default)]
    pub organizations: Option<Vec<String>>,
    #[serde(default)]
    pub password: Option<SecretString>,
}

impl NewUserRequest {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let request: Self = decode(&user_fields(value)?)?;
        check_user_role(request.user_role)?;
        if request.password.is_some() {
            return Err(ContractError::from(PASSWORD_UNSUPPORTED));
        }
        Ok(request)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateUserFields {
    #[serde(flatten)]
    pub base: GenerateRequestBase,
    #[serde(default)]
    pub password: Option<SecretString>,
    #[serde(default)]
    pub spend: Option<Amount>,
    #[serde(default)]
    pub metadata: Option<Payload>,
    #[serde(default)]
    pub user_alias: Option<String>,
    #[serde(default)]
    pub user_role: Option<UserRole>,
    #[serde(default)]
    pub max_budget: Option<Amount>,
    // pydantic's model_fields_set for the six fields above, which tells an omitted field from an explicit null.
    #[serde(skip)]
    pub supplied: BTreeSet<String>,
}

impl UpdateUserFields {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = user_fields(value)?;
        let mut request: Self = decode(&fields)?;
        check_user_role(request.user_role)?;
        request.supplied = supplied_fields(&fields, &USER_FIELDS);
        Ok(request)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateUserRequest {
    #[serde(flatten)]
    pub fields: UpdateUserFields,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub user_email: Option<String>,
}

impl UpdateUserRequest {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = user_fields(value)?;
        check_user_info(&fields)?;
        let mut request: Self = decode(&fields)?;
        request.fields.supplied = supplied_fields(&fields, &USER_FIELDS);
        check_user_role(request.fields.user_role)?;
        Ok(request)
    }
}

// GenerateRequestBase's max_budget validator runs before decoding on both user requests, so a blank string becomes null.
fn user_fields(value: &JsonValue) -> Result<Fields, ContractError> {
    let mut fields = require_object(value)?;
    blank_to_null(&mut fields, &["max_budget"]);
    Ok(fields)
}

// The before-validator tests `is None`, so an empty string selects the user and a null does not.
fn check_user_info(fields: &Fields) -> Result<(), ContractError> {
    if is_set(fields, "user_id") || is_set(fields, "user_email") {
        return Ok(());
    }
    Err(ContractError::from(USER_SELECTOR))
}

// LiteLLM's two user requests accept four of the seven roles; the literal rejects the other three.
fn check_user_role(role: Option<UserRole>) -> Result<(), ContractError> {
    match role {
        None
        | Some(
            UserRole::ProxyAdmin
            | UserRole::ProxyAdminViewer
            | UserRole::InternalUser
            | UserRole::InternalUserViewer,
        ) => Ok(()),
        Some(role @ (UserRole::OrgAdmin | UserRole::Team | UserRole::Customer)) => Err(
            ContractError::from(format!("user_role {role} is not allowed")),
        ),
    }
}

const fn default_auto_create_key() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: SecretString,
    pub new_password: SecretString,
}

impl ChangePasswordRequest {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        let request: Self = decode(&fields)?;
        Ok(request)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ChangePasswordResponse {
    pub user_id: String,
    pub message: String,
}

impl ChangePasswordResponse {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        let request: Self = decode(&fields)?;
        Ok(request)
    }
}

pub mod info;

mod tests;
