use betterlite_auth::auth_identity::roles::Role;

use crate::auth::auth_identity::AuthContext;

use super::error::RoiConnectionsError;

pub const READ_FORBIDDEN: &str = "Only proxy admins can access the ROI Calculator.";
pub const WRITE_FORBIDDEN: &str = "Only proxy admins can change ROI Calculator settings.";
pub const READ_ROLES: &[Role] = &[Role::PlatformAdmin, Role::PlatformViewer];
pub const WRITE_ROLES: &[Role] = &[Role::PlatformAdmin];
const MASTER_KEY_SUBJECT: &str = "master_key";

pub fn require_admin(
    caller: Option<&AuthContext>,
    roles: &[Role],
    forbidden: &'static str,
) -> Result<(), RoiConnectionsError> {
    let Some(context) = caller else {
        return Err(RoiConnectionsError::Unauthenticated);
    };
    let principal = &context.principal;
    let master = principal.subject == MASTER_KEY_SUBJECT;
    if master || principal.roles.iter().any(|role| roles.contains(role)) {
        return Ok(());
    }
    Err(RoiConnectionsError::Forbidden(forbidden))
}
