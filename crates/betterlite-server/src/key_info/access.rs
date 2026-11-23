use betterlite_auth::auth_team_lookup::lookup::TeamLookup;
use betterlite_auth::resolved_key::auth::{UserAPIKeyAuth, user_api_key_has_admin_view};
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use subtle::ConstantTimeEq as _;

use crate::team_access::caller::TeamCaller;
use crate::team_access::error::TeamAccessError;
use crate::team_access::key_team::user_belongs_to_keys_team;

mod tests;

// the UserAPIKeyAuth fields this check compares, read once from the key's serde form
#[derive(Deserialize)]
struct CallerFields {
    #[serde(default)]
    api_key: Option<SecretString>,
    #[serde(default)]
    user_id: Option<String>,
}

// _can_user_query_key_info: an admin, the key's own api_key, its owner, or a member of its team
pub async fn can_user_query_key_info(
    caller: &UserAPIKeyAuth,
    key: Option<&str>,
    key_row: &LiteLLMVerificationToken,
    teams: &TeamLookup,
) -> Result<bool, TeamAccessError> {
    if user_api_key_has_admin_view(caller) {
        return Ok(true);
    }
    let fields: CallerFields = serde_json::to_value(caller)
        .and_then(serde_json::from_value)
        .map_err(TeamAccessError::Caller)?;
    if api_key_matches(fields.api_key.as_ref(), key) || key_row.user_id == fields.user_id {
        return Ok(true);
    }
    let team_caller = TeamCaller::try_from(caller)?;
    user_belongs_to_keys_team(&team_caller, key_row.team_id.as_deref(), teams).await
}

// LiteLLM compares the two api_key options with ==; the stored hash still never leaves SecretString
fn api_key_matches(caller_key: Option<&SecretString>, query_key: Option<&str>) -> bool {
    match (caller_key, query_key) {
        (Some(stored), Some(requested)) => bool::from(
            stored
                .expose_secret()
                .as_bytes()
                .ct_eq(requested.as_bytes()),
        ),
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}
