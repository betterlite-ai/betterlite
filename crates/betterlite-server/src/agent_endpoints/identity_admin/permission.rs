use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use super::error::{Detail, IdentityAdminError};
use crate::plugin_routes::routes::PROXY_ADMIN_ROLE;
use crate::team_access::caller::TeamCaller;

mod tests;

// ast-grep-ignore: no-long-names
pub(crate) fn check_agent_management_permission(
    auth: &UserAPIKeyAuth,
) -> Result<(), IdentityAdminError> {
    let caller = TeamCaller::try_from(auth).map_err(|_error| IdentityAdminError {
        status: 500,
        detail: Detail::Text(String::from("the caller's role could not be read")),
    })?;
    let role = caller.user_role.map(|role| role.to_string());
    check_agent_management_role(role.as_deref())
}

// a JWT caller's principal carries its role while its key may not, and caller_identity falls back to the principal's roles
pub(crate) fn key_has_role(auth: &UserAPIKeyAuth) -> bool {
    match TeamCaller::try_from(auth) {
        Ok(caller) => caller.user_role.is_some(),
        Err(_error) => false,
    }
}

// LiteLLM compares user_api_key_dict.user_role, which the master key and JWT callers carry on the principal
pub(crate) fn check_agent_management_role(role: Option<&str>) -> Result<(), IdentityAdminError> {
    if role == Some(PROXY_ADMIN_ROLE) {
        return Ok(());
    }
    Err(IdentityAdminError {
        status: 403,
        detail: Detail::Object {
            error: format!(
                "Only proxy admins can create, update, or delete agents. Your role={}",
                role.unwrap_or("None")
            ),
        },
    })
}
