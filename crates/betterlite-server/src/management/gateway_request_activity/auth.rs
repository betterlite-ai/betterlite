use betterlite_auth::auth_identity::roles::Role;

use crate::auth::auth_identity::AuthContext;

use super::error::ActivityError;

pub(super) const FORBIDDEN: &str =
    "Only proxy admin roles can view gateway request counts across the deployment";
pub(super) const ROLES: &[Role] = &[Role::PlatformAdmin, Role::PlatformViewer];

pub(super) fn require_admin(caller: Option<&AuthContext>) -> Result<(), ActivityError> {
    let Some(context) = caller else {
        return Err(ActivityError::Unauthenticated);
    };
    let principal = &context.principal;
    let allowed = ROLES.iter().any(|role| principal.roles.contains(role));
    if !allowed {
        return Err(ActivityError::Forbidden(FORBIDDEN));
    }
    Ok(())
}
