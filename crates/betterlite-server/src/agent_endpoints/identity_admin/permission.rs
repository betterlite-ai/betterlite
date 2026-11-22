use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::user_roles::UserRole;
use serde::Deserialize;

use super::error::{Detail, IdentityAdminError};

mod tests;

// the UserAPIKeyAuth field this check reads, taken from the key's serde form as team access does
#[derive(Deserialize)]
struct ManagementCaller {
    #[serde(default)]
    user_role: Option<UserRole>,
}

// ast-grep-ignore: no-long-names
pub(crate) fn check_agent_management_permission(
    auth: &UserAPIKeyAuth,
) -> Result<(), IdentityAdminError> {
    let caller: ManagementCaller = serde_json::to_value(auth)
        .and_then(serde_json::from_value)
        .map_err(|_error| IdentityAdminError {
            status: 500,
            detail: Detail::Text(String::from("the caller's role could not be read")),
        })?;
    if caller.user_role == Some(UserRole::ProxyAdmin) {
        return Ok(());
    }
    let role = match caller.user_role {
        Some(role) => role.to_string(),
        None => String::from("None"),
    };
    Err(IdentityAdminError {
        status: 403,
        detail: Detail::Object {
            error: format!(
                "Only proxy admins can create, update, or delete agents. Your role={role}"
            ),
        },
    })
}
