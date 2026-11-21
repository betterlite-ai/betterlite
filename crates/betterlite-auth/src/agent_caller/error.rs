use crate::auth_team_lookup::error::TeamLookupError;
use crate::auth_user_lookup::error::UserLookupError;
use crate::resolved_key::auth::AuthError;

#[derive(Debug, thiserror::Error)]
pub enum AgentCallerError {
    #[error("the agent caller team {team_id} could not be loaded")]
    Team {
        team_id: String,
        #[source]
        source: Box<TeamLookupError>,
    },
    #[error("the agent caller user {user_id} could not be loaded")]
    User {
        user_id: String,
        #[source]
        source: Box<UserLookupError>,
    },
    #[error("the stand-in record for the agent caller could not be built")]
    Record(#[source] AuthError),
}
