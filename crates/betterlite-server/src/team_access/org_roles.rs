use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_user_lookup::lookup::{UserLookup, UserLookupRequest};
use betterlite_auth::user_roles::UserRole;

use super::access::{OrgRoles, TeamAccess};
use super::error::TeamAccessError;

// users/service.py PrismaOrgRoles: the caller's user row, never created here, and its memberships
pub struct LookupOrgRoles {
    pub users: Arc<UserLookup>,
}

#[async_trait]
impl OrgRoles for LookupOrgRoles {
    async fn is_org_admin(
        &self,
        user_id: &str,
        organization_id: &str,
    ) -> Result<bool, TeamAccessError> {
        let request = UserLookupRequest {
            user_id: Some(user_id),
            sso_user_id: None,
            user_email: None,
            user_id_upsert: false,
            check_db_only: None,
        };
        let user = self
            .users
            .get_user_object(request)
            .await
            .map_err(|error| TeamAccessError::OrgRoles(Box::new(error)))?;
        let admin = UserRole::OrgAdmin.to_string();
        let mut memberships = user
            .iter()
            .flat_map(|found| found.organization_memberships.iter().flatten());
        Ok(memberships.any(|membership| {
            membership.organization_id == organization_id
                && membership.user_role.as_deref() == Some(admin.as_str())
        }))
    }
}

// dependencies.py get_team_access: TeamAccess over the user lookup
impl From<Arc<UserLookup>> for TeamAccess {
    fn from(users: Arc<UserLookup>) -> Self {
        Self {
            org_roles: Arc::new(LookupOrgRoles { users }),
        }
    }
}
