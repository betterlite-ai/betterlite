use betterlite_auth::user_roles::UserRole;

use crate::team_access::caller::TeamCaller;

// _is_admin_view_safe: only the two proxy admin roles see every tenant's spend logs
#[must_use]
pub const fn is_admin_view_safe(caller: &TeamCaller) -> bool {
    match caller.user_role {
        Some(UserRole::ProxyAdmin | UserRole::ProxyAdminViewer) => true,
        Some(
            UserRole::OrgAdmin
            | UserRole::InternalUser
            | UserRole::InternalUserViewer
            | UserRole::Team
            | UserRole::Customer,
        )
        | None => false,
    }
}

// _can_user_view_spend_log: the list routes scope internal users to their own rows
#[must_use]
pub const fn can_user_view_spend_log(caller: &TeamCaller) -> bool {
    match caller.user_role {
        Some(UserRole::InternalUser | UserRole::InternalUserViewer) => caller.user_id.is_some(),
        Some(
            UserRole::ProxyAdmin
            | UserRole::ProxyAdminViewer
            | UserRole::OrgAdmin
            | UserRole::Team
            | UserRole::Customer,
        )
        | None => false,
    }
}
