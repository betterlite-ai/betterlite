use std::convert::Infallible;

use async_trait::async_trait;
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_server::model_write_fields::member::{MemberRouterWrite, MemberWrite};
use betterlite_server::model_write_fields::teams::TeamRows;
use betterlite_server::team_access::access::OrgRoles;
use betterlite_server::team_access::caller::TeamCaller;
use betterlite_server::team_access::error::TeamAccessError;
use tokio::sync::Mutex;

// the team table the case holds, recording each team_id the check reads
pub(super) struct CaseTeams {
    pub(super) teams: Vec<LiteLLMTeamTable>,
    pub(super) reads: Mutex<Vec<String>>,
}

#[async_trait]
impl TeamRows for CaseTeams {
    async fn find_team(&self, team_id: &str) -> Result<Option<LiteLLMTeamTable>, TeamQueryError> {
        self.reads.lock().await.push(team_id.to_owned());
        Ok(self
            .teams
            .iter()
            .find(|stored| stored.fields.base.team_id == team_id)
            .cloned())
    }
}

// TEAM_ADMIN_ONLY never asks the organization, so no case reaches this
pub(super) struct NoOrgAdmins;

#[async_trait]
impl OrgRoles for NoOrgAdmins {
    async fn is_org_admin(
        &self,
        _user_id: &str,
        _organization_id: &str,
    ) -> Result<bool, TeamAccessError> {
        Ok(false)
    }
}

// the oracle's LiteLLM has no llm_router, so a member router write finds no policy on either side
pub(super) struct Unregistered;

#[async_trait]
impl MemberRouterWrite for Unregistered {
    type Patch = ();
    type Stored = ();
    type Grant = ();
    type Refusal = Infallible;

    async fn authorize(
        &self,
        _caller: &TeamCaller,
        _team: &LiteLLMTeamTable,
        _write: MemberWrite<'_, (), ()>,
    ) -> Result<(), Infallible> {
        Ok(())
    }
}
