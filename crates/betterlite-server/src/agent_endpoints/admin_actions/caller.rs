use betterlite_auth::auth_identity::records::Principal;
use secrecy::SecretString;
use serde::Deserialize;

use super::error::unexpected_role;
use crate::agent_endpoints::identity_admin::error::IdentityAdminError;
use crate::plugin_routes::routes::caller_identity;

// the UserAPIKeyAuth fields these routes read: the caller's user_id, its user_role and the key's safe provenance hash
#[derive(Debug)]
pub(super) struct AdminCaller {
    pub(super) user_id: Option<String>,
    pub(super) user_role: Option<String>,
    pub(super) api_key: Option<SecretString>,
}

pub(super) fn admin_caller(principal: &Principal) -> Result<AdminCaller, IdentityAdminError> {
    let identity = caller_identity(principal).map_err(|_error| unexpected_role())?;
    let key = key_view(principal).map_err(|_error| unexpected_role())?;
    Ok(AdminCaller {
        user_id: identity.user_id,
        user_role: identity.user_role,
        api_key: key.api_key,
    })
}

// the key's serde form exposes api_key as its safe provenance hash, the value LiteLLM puts on AuditLogEntry.changed_by_api_key
fn key_view(principal: &Principal) -> Result<KeyView, serde_json::Error> {
    let Some(key) = principal.source_key.0.as_ref() else {
        return Ok(KeyView::default());
    };
    serde_json::from_value(serde_json::to_value(key)?)
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct KeyView {
    api_key: Option<SecretString>,
}
