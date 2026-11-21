use std::error::Error;

use async_trait::async_trait;
use betterlite_auth::user_roles::UserRole;
use betterlite_config::sections::management_rbac::Settings;

use super::{FeatureAccessError, FeatureName};
use crate::management_admins::scope::AdminCaller;

// B863's _user_has_admin_privileges as the guard calls it, with a failure the guard passes on uncaught
#[async_trait]
pub trait AdminPrivileges: Send + Sync {
    type Failure: Error + Send + 'static;

    async fn has_admin_privileges(&self, caller: &AdminCaller<'_>) -> Result<bool, Self::Failure>;
}

// check_feature_access_for_user: proxy admins before any setting, then the disable switch, then the admin exemption
pub async fn check_feature_access<Privileges>(
    role: Option<UserRole>,
    user_id: Option<&str>,
    feature: FeatureName,
    settings: &Settings,
    privileges: &Privileges,
) -> Result<(), FeatureAccessError<Privileges::Failure>>
where
    Privileges: AdminPrivileges + ?Sized,
{
    match role {
        Some(UserRole::ProxyAdmin | UserRole::ProxyAdminViewer) => return Ok(()),
        Some(
            UserRole::OrgAdmin
            | UserRole::InternalUser
            | UserRole::InternalUserViewer
            | UserRole::Team
            | UserRole::Customer,
        )
        | None => {}
    }
    let switches = match feature {
        FeatureName::Agents => settings.agents,
        FeatureName::VectorStores => settings.vector_stores,
    };
    if !switches.disabled {
        return Ok(());
    }
    if switches.team_admins_allowed {
        // B863 tests the role for proxy_admin first, and the match above already returned for it
        let caller = AdminCaller {
            user_id,
            proxy_admin: false,
        };
        let granted = privileges
            .has_admin_privileges(&caller)
            .await
            .map_err(FeatureAccessError::Privileges)?;
        if granted {
            return Ok(());
        }
    }
    Err(FeatureAccessError::Disabled(feature))
}
