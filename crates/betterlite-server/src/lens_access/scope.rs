use std::collections::BTreeMap;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_lens::models::settings::Scope;
use serde::Deserialize;

use crate::lens_access::error::LensAccessError;
use crate::plugin_routes::routes::caller_identity;

// the UserAPIKeyAuth fields the Lens routes read, taken once from the key's serde form
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct KeyCaller {
    pub(crate) user_role: Option<UserRole>,
    pub(crate) team_id: Option<String>,
    pub(crate) models: Vec<String>,
}

pub(crate) fn key_caller(auth: &UserAPIKeyAuth) -> Result<KeyCaller, LensAccessError> {
    serde_json::to_value(auth)
        .and_then(serde_json::from_value)
        .map_err(LensAccessError::Caller)
}

// the resolved key LiteLLM's user_api_key_auth hands the handlers, rebuilt from B003's principal
pub(crate) fn caller_key(principal: &Principal) -> Result<UserAPIKeyAuth, LensAccessError> {
    if let Some(key) = principal.source_key.0.as_ref() {
        return Ok(key.clone());
    }
    let identity = caller_identity(principal).map_err(LensAccessError::Caller)?;
    let mut fields = BTreeMap::new();
    if let Some(role) = identity.user_role {
        fields.insert("user_role".to_owned(), JsonData::String(role));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).map_err(LensAccessError::CallerKey)
}

pub(crate) fn user_scope(auth: &UserAPIKeyAuth, write: bool) -> Result<Scope, LensAccessError> {
    let role = key_caller(auth)?.user_role;
    if write && role != Some(UserRole::ProxyAdmin) {
        return Err(LensAccessError::WriteDenied);
    }
    match role {
        Some(UserRole::ProxyAdmin | UserRole::ProxyAdminViewer) => Ok(Scope {
            team_id: String::new(),
            api_key_hash: String::new(),
            all_teams: true,
        }),
        Some(
            UserRole::OrgAdmin
            | UserRole::InternalUser
            | UserRole::InternalUserViewer
            | UserRole::Team
            | UserRole::Customer,
        )
        | None => Err(LensAccessError::AccessDenied),
    }
}

mod tests;
