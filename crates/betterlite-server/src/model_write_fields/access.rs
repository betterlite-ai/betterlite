use betterlite_auth::user_roles::UserRole;
use betterlite_db::team_rows::team::{LiteLLMTeamTable, TeamBase};

use super::error::{MemberCallError, ModelWriteError};
use super::member::{MemberRouterWrite, MemberWrite, ModelWriteGrant};
use super::teams::TeamRows;
use crate::team_access::access::{TEAM_ADMIN_ONLY, TeamAccess, is_team_admin};
use crate::team_access::caller::TeamCaller;

// model deletion may act on a model whose team is gone, but only as a proxy admin
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingTeam {
    Refuse,
    AdminMayAct,
}

pub struct ModelWriteAccess<'access> {
    pub teams: &'access dyn TeamRows,
    pub team_access: &'access TeamAccess,
}

struct Owner<'id> {
    team_id: &'id str,
    team: Option<LiteLLMTeamTable>,
}

// can_user_make_team_model_call without its premium_user gate, since betterlite has no paid tier
pub fn can_make_team_model_call(
    team_id: &str,
    caller: &TeamCaller,
    team: Option<&TeamBase<String>>,
) -> Result<(), ModelWriteError> {
    if caller.user_role == Some(UserRole::ProxyAdmin)
        || team.is_some_and(|found| is_team_admin(caller, found))
    {
        return Ok(());
    }
    Err(ModelWriteError::NotTeamAdmin {
        team_id: team_id.to_owned(),
        key_team: caller.team_id.clone(),
    })
}

impl ModelWriteAccess<'_> {
    // allow_team_model_action: a write that names no team needs no team authority
    pub async fn allow_team_model_action(
        &self,
        caller: &TeamCaller,
        team_id: Option<&str>,
    ) -> Result<(), ModelWriteError> {
        let Some(owner) = team_id else {
            return Ok(());
        };
        let team = self
            .teams
            .find_team(owner)
            .await
            .map_err(ModelWriteError::TeamRead)?
            .ok_or_else(|| ModelWriteError::TeamMissing {
                team_id: owner.to_owned(),
            })?;
        can_make_team_model_call(owner, caller, Some(&team.fields.base))
    }

    // can_user_make_model_call without a member operation, as model deletion and health checks call it
    pub async fn can_make_model_call(
        &self,
        caller: &TeamCaller,
        team_id: Option<&str>,
        missing_team: MissingTeam,
    ) -> Result<(), ModelWriteError> {
        let Some(owner) = self.read_owner(caller, team_id).await? else {
            return Ok(());
        };
        match (owner.team, missing_team) {
            (Some(team), MissingTeam::Refuse | MissingTeam::AdminMayAct) => {
                can_make_team_model_call(owner.team_id, caller, Some(&team.fields.base))
            }
            (None, MissingTeam::AdminMayAct) if caller.user_role == Some(UserRole::ProxyAdmin) => {
                Ok(())
            }
            (None, MissingTeam::AdminMayAct) => Err(ModelWriteError::OrphanedModel),
            (None, MissingTeam::Refuse) => Err(ModelWriteError::TeamMissing {
                team_id: owner.team_id.to_owned(),
            }),
        }
    }

    // can_user_make_model_call for a create or update: a member who is no team admin goes to the member router policy
    pub async fn can_make_member_write<Hook>(
        &self,
        caller: &TeamCaller,
        team_id: Option<&str>,
        policy: Option<&Hook>,
        write: MemberWrite<'_, Hook::Patch, Hook::Stored>,
    ) -> Result<ModelWriteGrant<Hook::Grant>, MemberCallError<Hook::Refusal>>
    where
        Hook: MemberRouterWrite,
    {
        let Some(owner) = self.read_owner(caller, team_id).await? else {
            return Ok(ModelWriteGrant::Authorized);
        };
        let team = owner.team.ok_or_else(|| ModelWriteError::TeamMissing {
            team_id: owner.team_id.to_owned(),
        })?;
        let is_admin = self
            .team_access
            .allows(caller, &team.fields.base, &TEAM_ADMIN_ONLY)
            .await
            .map_err(ModelWriteError::TeamAccess)?;
        if is_admin {
            return Ok(ModelWriteGrant::Authorized);
        }
        let member_router = policy.ok_or(ModelWriteError::MemberRouterUnavailable)?;
        let grant = member_router
            .authorize(caller, &team, write)
            .await
            .map_err(MemberCallError::Refused)?;
        Ok(ModelWriteGrant::MemberRouter(grant))
    }

    // view-only callers stop first, and a model that names no team needs a proxy admin
    async fn read_owner<'id>(
        &self,
        caller: &TeamCaller,
        team_id: Option<&'id str>,
    ) -> Result<Option<Owner<'id>>, ModelWriteError> {
        if matches!(
            caller.user_role,
            Some(UserRole::ProxyAdminViewer | UserRole::InternalUserViewer)
        ) {
            return Err(ModelWriteError::ViewOnly);
        }
        let Some(owner) = team_id else {
            if caller.user_role == Some(UserRole::ProxyAdmin) {
                return Ok(None);
            }
            return Err(ModelWriteError::NotProxyAdmin);
        };
        let team = self
            .teams
            .find_team(owner)
            .await
            .map_err(ModelWriteError::TeamRead)?;
        Ok(Some(Owner {
            team_id: owner,
            team,
        }))
    }
}
