use betterlite_auth::resolved_key::auth::{UserAPIKeyAuth, is_team_service_account};
use betterlite_auth::user_roles::UserRole;
use serde::Deserialize;

use super::error::TeamAccessError;

// the UserAPIKeyAuth fields team access reads, taken once from the key's serde form
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TeamCaller {
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub user_role: Option<UserRole>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(skip)]
    pub team_service_account: bool,
}

impl TryFrom<&UserAPIKeyAuth> for TeamCaller {
    type Error = TeamAccessError;

    fn try_from(auth: &UserAPIKeyAuth) -> Result<Self, Self::Error> {
        let mut caller: Self = serde_json::to_value(auth)
            .and_then(serde_json::from_value)
            .map_err(TeamAccessError::Caller)?;
        caller.team_service_account = is_team_service_account(auth);
        Ok(caller)
    }
}
