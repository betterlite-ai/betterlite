use async_trait::async_trait;

use crate::spend::visibility::error::VisibilityError;
use crate::spend::visibility::owners::OwnerPair;
use crate::spend::visibility::service::SpendLogVisibility;
use crate::team_access::caller::TeamCaller;

// B1704's checks as the list route needs them; the execute step fails closed when one is absent
#[async_trait]
pub trait LogListScope: Send + Sync {
    async fn assert_can_view_lookup(
        &self,
        caller: &TeamCaller,
        request_id: &str,
    ) -> Result<(), VisibilityError>;
    async fn can_view_team(
        &self,
        caller: &TeamCaller,
        team_id: &str,
    ) -> Result<bool, VisibilityError>;
    async fn visible_team_ids(&self, caller: &TeamCaller) -> Vec<String>;
    async fn can_view_row_owner(
        &self,
        caller: &TeamCaller,
        owner: &OwnerPair,
    ) -> Result<bool, VisibilityError>;
}

// every method hands the question to B1704's own check; _can_team_member_view_log is _user_can_view_spend_log_owner without an owner
#[async_trait]
impl LogListScope for SpendLogVisibility {
    #[cfg_attr(test, mutants::skip)]
    async fn assert_can_view_lookup(
        &self,
        caller: &TeamCaller,
        request_id: &str,
    ) -> Result<(), VisibilityError> {
        self.assert_can_view_request_id(caller, request_id).await
    }

    #[cfg_attr(test, mutants::skip)]
    async fn can_view_team(
        &self,
        caller: &TeamCaller,
        team_id: &str,
    ) -> Result<bool, VisibilityError> {
        let team_owner = OwnerPair {
            user: None,
            team_id: Some(team_id.to_owned()),
        };
        self.can_view_owner(caller, &team_owner).await
    }

    #[cfg_attr(test, mutants::skip)]
    async fn visible_team_ids(&self, caller: &TeamCaller) -> Vec<String> {
        self.permitted_teams_or_empty(caller).await
    }

    #[cfg_attr(test, mutants::skip)]
    async fn can_view_row_owner(
        &self,
        caller: &TeamCaller,
        owner: &OwnerPair,
    ) -> Result<bool, VisibilityError> {
        self.can_view_owner(caller, owner).await
    }
}
