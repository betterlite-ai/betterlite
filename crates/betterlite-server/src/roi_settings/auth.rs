use betterlite_auth::auth_identity::roles::Role;

use crate::auth::auth_identity::AuthContext;

use super::error::RoiSettingsError;

pub(super) const READ_FORBIDDEN: &str = "Only proxy admins can access the ROI Calculator.";
pub(super) const WRITE_FORBIDDEN: &str = "Only proxy admins can change ROI Calculator settings.";
pub(super) const READ_ROLES: &[Role] = &[Role::PlatformAdmin, Role::PlatformViewer];
pub(super) const WRITE_ROLES: &[Role] = &[Role::PlatformAdmin];

pub(super) fn require_admin(
    caller: Option<&AuthContext>,
    roles: &[Role],
    forbidden: &'static str,
) -> Result<(), RoiSettingsError> {
    let Some(context) = caller else {
        return Err(RoiSettingsError::Unauthenticated);
    };
    let principal = &context.principal;
    let allowed = roles.iter().any(|role| principal.roles.contains(role));
    if !allowed {
        return Err(RoiSettingsError::Forbidden(forbidden));
    }
    Ok(())
}
