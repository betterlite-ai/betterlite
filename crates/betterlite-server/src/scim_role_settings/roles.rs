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
    match settings.default_user_role {
        Some(configured) => user_role(configured),
        None => UserRole::InternalUserViewer,
    }
}

// None means no admin group is configured, so the caller leaves the stored role untouched
#[must_use]
pub fn resolve_scim_user_role(
    groups: &[UserGroup],
    admin_group: Option<&str>,
    default_role: UserRole,
) -> Option<UserRole> {
    let Some(admin) = admin_group else {
        return None;
    };
    Some(role_from_groups(groups, admin, default_role))
}

// a group grants admin by its value or by the display name a team alias supplies
fn role_from_groups(groups: &[UserGroup], admin_group: &str, default_role: UserRole) -> UserRole {
    for group in groups {
        if group.value == admin_group || group.display.as_deref() == Some(admin_group) {
            return UserRole::ProxyAdmin;
        }
    }
    default_role
}
