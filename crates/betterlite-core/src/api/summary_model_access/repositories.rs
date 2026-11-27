use std::fmt::Display;
use std::future::Future;

use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::project::ProjectTable;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;

// get_user_object, get_project_object and get_team_membership behind one seam; a row the database lacks is None
pub trait SummaryAccessRepositories: Sync {
    type Failure: Display;

    // compact.py reads the user row with user_id_upsert=False, so this lookup never creates one
    fn user(
        &self,
        user_id: &str,
        user_id_upsert: bool,
    ) -> impl Future<Output = Result<Option<LiteLLMUserTable>, Self::Failure>> + Send;

    fn project(
        &self,
        project_id: &str,
    ) -> impl Future<Output = Result<Option<ProjectTable>, Self::Failure>> + Send;

    fn membership(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> impl Future<Output = Result<Option<LiteLLMTeamMembership>, Self::Failure>> + Send;
}
