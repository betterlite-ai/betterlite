use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::effective_user_role;

use crate::plugin_routes::routes::caller_identity;
use crate::team_access::caller::TeamCaller;

// user_api_key_dict: the id and role the auth dependency publishes, with an unknown role falling back like get_user_role
pub(super) fn team_caller(principal: &Principal) -> Result<TeamCaller, serde_json::Error> {
    let identity = caller_identity(principal)?;
    Ok(TeamCaller {
        user_id: identity.user_id,
        user_role: Some(effective_user_role(identity.user_role.as_deref())),
        team_id: None,
        team_service_account: false,
    })
}
