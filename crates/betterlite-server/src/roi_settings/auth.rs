use betterlite_auth::auth_identity::roles::Role;

use crate::auth::auth_identity::AuthContext;

use super::error::RoiSettingsError;

pub(super) const READ_FORBIDDEN: &str = "Only proxy admins can access the ROI Calculator.";
pub(super) const WRITE_FORBIDDEN: &str = "Only proxy admins can change ROI Calculator settings.";
const MASTER_KEY_SUBJECT: &str = "master_key";

pub(super) fn read_admin(caller: Option<&AuthContext>) -> Result<(), RoiSettingsError> {
    let Some(context) = caller else {
        return Err(RoiSettingsError::Unauthenticated);
    };
    let principal = &context.principal;
    let allowed = principal.subject == MASTER_KEY_SUBJECT
        || principal.roles.contains(&Role::PlatformAdmin)
        || principal.roles.contains(&Role::PlatformViewer);
    if !allowed {
        return Err(RoiSettingsError::Forbidden(READ_FORBIDDEN));
    }
    Ok(())
}

pub(super) fn write_admin(caller: Option<&AuthContext>) -> Result<(), RoiSettingsError> {
    let Some(context) = caller else {
        return Err(RoiSettingsError::Unauthenticated);
    };
    let principal = &context.principal;
    let allowed =
        principal.subject == MASTER_KEY_SUBJECT || principal.roles.contains(&Role::PlatformAdmin);
    if !allowed {
        return Err(RoiSettingsError::Forbidden(WRITE_FORBIDDEN));
    }
    Ok(())
}
