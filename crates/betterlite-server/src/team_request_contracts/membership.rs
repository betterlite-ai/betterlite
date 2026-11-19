use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamMemberInfoResponse {
    #[serde(flatten)]
    pub membership: LiteLLMTeamMembership,
    pub role: Option<String>,
    pub user_email: Option<String>,
    pub team_alias: Option<String>,
}
