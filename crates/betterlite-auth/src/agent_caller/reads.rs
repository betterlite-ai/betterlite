use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;

use crate::auth_team_lookup::error::TeamLookupError;
use crate::auth_user_lookup::error::UserLookupError;

// The server backs this with B737's get_team_object and B736's get_user_object without upsert
pub trait CallerReads: Sync {
    fn team(
        &self,
        team_id: &str,
    ) -> impl Future<Output = Result<LiteLLMTeamTableCachedObj, TeamLookupError>> + Send;

    fn user(
        &self,
        user_id: &str,
    ) -> impl Future<Output = Result<Option<LiteLLMUserTable>, UserLookupError>> + Send;
}
