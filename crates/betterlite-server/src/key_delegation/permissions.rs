use betterlite_auth::user_roles::UserRole;
use betterlite_db::identity_rows::Payload;

use crate::key_delegation::caller::Caller;
use crate::key_delegation::error::DelegationError;
use crate::key_delegation::request::Sent;

// Permissions carry model and MCP grants, so a non-admin may not send the field at all, even as null or {}.
pub fn check_permissions_access(
    permissions: &Sent<Payload>,
    caller: Caller,
) -> Result<(), DelegationError> {
    let nonempty = permissions
        .value
        .as_ref()
        .is_some_and(|payload| !payload.0.is_empty());
    if !permissions.supplied && !nonempty {
        return Ok(());
    }
    if caller.role == Some(UserRole::ProxyAdmin) {
        return Ok(());
    }
    Err(DelegationError::Permissions)
}
