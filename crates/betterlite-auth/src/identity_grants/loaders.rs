use async_trait::async_trait;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;

use crate::auth_team_lookup::error::TeamLookupError;
use crate::auth_team_lookup::lookup::{TeamLookup, TeamRequest};
use crate::auth_team_lookup::team::get_team_object;
use crate::auth_user_lookup::error::UserLookupError;
use crate::auth_user_lookup::lookup::{UserLookup, UserLookupRequest};
use crate::membership_load::MembershipLookup;
use crate::membership_load::error::MembershipLoadError;

// Each reader owns its database, cache and background tasks, so a load passes only the request.
#[async_trait]
pub trait UserLoader: Send + Sync {
    async fn load_user(
        &self,
        request: UserLookupRequest<'_>,
    ) -> Result<Option<LiteLLMUserTable>, UserLookupError>;
}

#[async_trait]
pub trait TeamLoader: Send + Sync {
    async fn load_team(
        &self,
        request: TeamRequest<'_>,
    ) -> Result<LiteLLMTeamTableCachedObj, TeamLookupError>;
}

#[async_trait]
pub trait MembershipLoader: Send + Sync {
    async fn load_membership(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError>;
}

#[async_trait]
impl UserLoader for UserLookup {
    async fn load_user(
        &self,
        request: UserLookupRequest<'_>,
    ) -> Result<Option<LiteLLMUserTable>, UserLookupError> {
        self.get_user_object(request).await
    }
}

#[async_trait]
impl TeamLoader for TeamLookup {
    async fn load_team(
        &self,
        request: TeamRequest<'_>,
    ) -> Result<LiteLLMTeamTableCachedObj, TeamLookupError> {
        get_team_object(self, request).await
    }
}

#[async_trait]
impl MembershipLoader for MembershipLookup {
    async fn load_membership(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        self.get_team_membership(Some(user_id), Some(team_id)).await
    }
}
