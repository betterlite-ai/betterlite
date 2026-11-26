use std::collections::BTreeMap;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;

use super::error::ManagementReadError;
use crate::plugin_routes::routes::caller_identity;

// the authenticated caller as LiteLLM's user_api_key_auth dependency hands it to the handler
#[derive(Debug, Clone)]
pub struct CallerView {
    pub key: UserAPIKeyAuth,
    pub role: Option<UserRole>,
    pub user_id: Option<String>,
}

impl CallerView {
    pub fn from_principal(principal: &Principal) -> Result<Self, ManagementReadError> {
        let identity =
            caller_identity(principal).map_err(|_error| ManagementReadError::Internal {
                message: "the caller identity could not be read".to_owned(),
            })?;
        let role = identity
            .user_role
            .as_deref()
            .and_then(|text| text.parse::<UserRole>().ok());
        let key = match principal.source_key.0.as_ref() {
            Some(key) => key.clone(),
            None => construct_key(identity.user_id.as_deref(), identity.user_role.as_deref())?,
        };
        Ok(Self {
            key,
            role,
            user_id: identity.user_id,
        })
    }
}

// the master key and a principal without a stored key are rebuilt as the credential the routes read
fn construct_key(
    user_id: Option<&str>,
    user_role: Option<&str>,
) -> Result<UserAPIKeyAuth, ManagementReadError> {
    let mut fields = BTreeMap::new();
    if let Some(user_id) = user_id {
        fields.insert("user_id".to_owned(), JsonData::String(user_id.to_owned()));
    }
    if let Some(user_role) = user_role {
        fields.insert(
            "user_role".to_owned(),
            JsonData::String(user_role.to_owned()),
        );
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).map_err(|_error| {
        ManagementReadError::Internal {
            message: "the caller identity could not be built".to_owned(),
        }
    })
}
