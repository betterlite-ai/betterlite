use betterlite_auth::user_roles::UserRole;
use betterlite_db::team_rows::team::{LiteLLMTeamTable, TeamBase};

use super::error::{MemberCallError, ModelWriteError};
use super::member::{MemberRouterWrite, MemberWrite, ModelWriteGrant};
use super::teams::TeamRows;
use crate::team_access::access::{TEAM_ADMIN_ONLY, TeamAccess, is_team_admin};
use crate::team_access::caller::TeamCaller;

// allow_team_model_action, or can_user_make_model_call without and with allow_missing_team as model deletion runs it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCall {
    TeamAction,
    Write,
    Delete,
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
    let allowed = caller.user_role == Some(UserRole::ProxyAdmin)
        || team.is_some_and(|found| is_team_admin(caller, found));
    allowed
        .then_some(())
        .ok_or_else(|| ModelWriteError::NotTeamAdmin {
            team_id: team_id.to_owned(),
            key_team: caller.team_id.clone(),
        })
}

impl ModelWriteAccess<'_> {
    // the check without a member operation, as team model actions, model deletion and health checks run it
    pub async fn can_make_model_call(
        &self,
        caller: &TeamCaller,
        team_id: Option<&str>,
        call: ModelCall,
    ) -> Result<(), ModelWriteError> {
        let Some(owner) = self.read_owner(caller, team_id, call).await? else {
            return Ok(());
        };
        match (owner.team, call) {
            (Some(team), ModelCall::TeamAction | ModelCall::Write | ModelCall::Delete) => {
                can_make_team_model_call(owner.team_id, caller, Some(&team.fields.base))
            }
            (None, ModelCall::Delete) if caller.user_role == Some(UserRole::ProxyAdmin) => Ok(()),
            (None, ModelCall::Delete) => Err(ModelWriteError::OrphanedModel),
            (None, ModelCall::TeamAction | ModelCall::Write) => Err(ModelWriteError::TeamMissing {
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
        let Some(owner) = self.read_owner(caller, team_id, ModelCall::Write).await? else {
            return Ok(ModelWriteGrant::Authorized);
        };
        let team = owner.team.ok_or_else(|| ModelWriteError::TeamMissing {
            team_id: owner.team_id.to_owned(),
        })?;
        let is_admin = self
            .team_access
            .allows(caller, &team.fields.base, &TEAM_ADMIN_ONLY)
            .await
            .map_err(|source| ModelWriteError::TeamAccess(Box::new(source)))?;
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

    // can_user_make_model_call stops view-only callers first and needs a proxy admin for a model naming no team
    async fn read_owner<'id>(
        &self,
        caller: &TeamCaller,
        team_id: Option<&'id str>,
        call: ModelCall,
    ) -> Result<Option<Owner<'id>>, ModelWriteError> {
        let screened = call != ModelCall::TeamAction;
        if screened
            && matches!(
                caller.user_role,
                Some(UserRole::ProxyAdminViewer | UserRole::InternalUserViewer)
            )
        {
            return Err(ModelWriteError::ViewOnly);
        }
        let Some(owner) = team_id else {
            if !screened || caller.user_role == Some(UserRole::ProxyAdmin) {
                return Ok(None);
            }
            return Err(ModelWriteError::NotProxyAdmin);
        };
        let team = self
            .teams
            .find_team(owner)
            .await
            .map_err(|source| ModelWriteError::TeamRead(Box::new(source)))?;
        Ok(Some(Owner {
            team_id: owner,
            team,
        }))
    }
}
