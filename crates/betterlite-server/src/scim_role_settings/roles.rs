use betterlite_auth::user_roles::UserRole;
use betterlite_config::scim_role_settings::{ScimUserRole, Settings};

use crate::scim_records::user::UserGroup;

// the four ScimUserRole literals map onto the proxy roles SCIM may assign
#[must_use]
pub const fn user_role(configured: ScimUserRole) -> UserRole {
    match configured {
        ScimUserRole::ProxyAdmin => UserRole::ProxyAdmin,
        ScimUserRole::ProxyAdminViewer => UserRole::ProxyAdminViewer,
        ScimUserRole::InternalUser => UserRole::InternalUser,
        ScimUserRole::InternalUserViewer => UserRole::InternalUserViewer,
    }
}

// _default_scim_user_role falls back to the viewer role when no user_role is configured
#[must_use]
pub const fn default_scim_user_role(settings: &Settings) -> UserRole {
    let Some(configured) = settings.default_user_role else {
        return UserRole::InternalUserViewer;
    };
    user_role(configured)
}

// None means no admin group is configured, so the caller leaves the stored role untouched
#[must_use]
pub fn resolve_scim_user_role(
    groups: &[UserGroup],
    admin_group: Option<&str>,
    default_role: UserRole,
) -> Option<UserRole> {
    let admin = admin_group?;
    let matched = groups
        .iter()
        .any(|group| group.value == admin || group.display.as_deref() == Some(admin));
    Some(if matched {
        UserRole::ProxyAdmin
    } else {
        default_role
    })
}
