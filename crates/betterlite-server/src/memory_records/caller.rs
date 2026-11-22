use betterlite_auth::resolved_key::auth::{UserAPIKeyAuth, user_api_key_has_admin_view};
use betterlite_auth::user_roles::UserRole;

use crate::team_access::caller::TeamCaller;
use crate::team_access::error::TeamAccessError;

// the UserAPIKeyAuth fields the memory helpers read, taken once so every helper sees one principal
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryCaller {
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub user_role: Option<UserRole>,
    pub service_account: bool,
    pub admin_view: bool,
}

impl TryFrom<&UserAPIKeyAuth> for MemoryCaller {
    type Error = TeamAccessError;

    fn try_from(auth: &UserAPIKeyAuth) -> Result<Self, Self::Error> {
        let caller = TeamCaller::try_from(auth)?;
        Ok(Self {
            user_id: caller.user_id,
            team_id: caller.team_id,
            user_role: caller.user_role,
            service_account: caller.team_service_account,
            admin_view: user_api_key_has_admin_view(auth),
        })
    }
}

// memory_endpoints.py _is_admin: only PROXY_ADMIN writes, while the admin view also covers the viewer
#[must_use]
pub const fn is_admin(caller: &MemoryCaller) -> bool {
    match caller.user_role {
        Some(UserRole::ProxyAdmin) => true,
        Some(
            UserRole::ProxyAdminViewer
            | UserRole::OrgAdmin
            | UserRole::InternalUser
            | UserRole::InternalUserViewer
            | UserRole::Team
            | UserRole::Customer,
        )
        | None => false,
    }
}
