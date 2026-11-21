use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::user_roles::UserRole;
use betterlite_db::team_rows::member::MemberRole;
use betterlite_db::team_rows::team::TeamBase;
use serde::{Deserialize, Serialize};

use super::caller::TeamCaller;
use super::error::TeamAccessError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamRole {
    ProxyAdmin,
    OrgAdmin,
    TeamAdmin,
}

pub const TEAM_ADMIN_ONLY: [TeamRole; 2] = [TeamRole::ProxyAdmin, TeamRole::TeamAdmin];
pub const TEAM_OR_ORG_ADMIN: [TeamRole; 3] = [
    TeamRole::ProxyAdmin,
    TeamRole::TeamAdmin,
    TeamRole::OrgAdmin,
];

// access.py OrgRoles: the organization read a TeamAccess is built with
#[async_trait]
pub trait OrgRoles: Send + Sync {
    async fn is_org_admin(
        &self,
        user_id: &str,
        organization_id: &str,
    ) -> Result<bool, TeamAccessError>;
}

pub struct TeamAccess {
    pub org_roles: Arc<dyn OrgRoles>,
}

impl TeamAccess {
    // a roster admin is checked before the organization, so only callers off the roster pay for the lookup
    pub async fn allows(
        &self,
        caller: &TeamCaller,
        team: &TeamBase<String>,
        allow: &[TeamRole],
    ) -> Result<bool, TeamAccessError> {
        if allow.contains(&TeamRole::ProxyAdmin) && caller.user_role == Some(UserRole::ProxyAdmin) {
            return Ok(true);
        }
        if allow.contains(&TeamRole::TeamAdmin) && is_team_admin(caller, team) {
            return Ok(true);
        }
        if !allow.contains(&TeamRole::OrgAdmin) {
            return Ok(false);
        }
        self.is_org_admin(caller, team).await
    }

    // the organization outranks the roster, so a caller holding both keeps unrestricted edits
    pub async fn strongest_role(
        &self,
        caller: &TeamCaller,
        team: &TeamBase<String>,
    ) -> Result<Option<TeamRole>, TeamAccessError> {
        if caller.user_role == Some(UserRole::ProxyAdmin) {
            return Ok(Some(TeamRole::ProxyAdmin));
        }
        if self.is_org_admin(caller, team).await? {
            return Ok(Some(TeamRole::OrgAdmin));
        }
        Ok(is_team_admin(caller, team).then_some(TeamRole::TeamAdmin))
    }

    async fn is_org_admin(
        &self,
        caller: &TeamCaller,
        team: &TeamBase<String>,
    ) -> Result<bool, TeamAccessError> {
        let user_id = caller.user_id.as_deref().filter(|id| !id.is_empty());
        let organization_id = team.organization_id.as_deref().filter(|id| !id.is_empty());
        let (Some(user), Some(organization)) = (user_id, organization_id) else {
            return Ok(false);
        };
        self.org_roles.is_org_admin(user, organization).await
    }
}

// any admin entry counts, unlike the first-match roster lookup a member's role comes from
#[must_use]
pub fn is_team_admin(caller: &TeamCaller, team: &TeamBase<String>) -> bool {
    team.members_with_roles.iter().any(|member| {
        member.role == MemberRole::Admin
            && member.base.user_id.is_some()
            && member.base.user_id == caller.user_id
    })
}
