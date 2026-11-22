use betterlite_auth::resolved_key::auth::{UserAPIKeyAuth, user_api_key_has_admin_view};
use betterlite_auth::user_roles::UserRole;

use crate::shadow_eval_validation::error::AccessError;
use crate::team_access::caller::TeamCaller;

// _require_admin_viewer: the list and detail endpoints serve both admin roles.
pub fn require_admin_viewer(auth: &UserAPIKeyAuth, action: &str) -> Result<(), AccessError> {
    if user_api_key_has_admin_view(auth) {
        return Ok(());
    }
    Err(AccessError::ViewerForbidden {
        action: action.to_owned(),
    })
}

// _require_admin_writer: starting and stopping a job is the full admin role only.
pub fn require_admin_writer(auth: &UserAPIKeyAuth, action: &str) -> Result<(), AccessError> {
    let caller = TeamCaller::try_from(auth).map_err(|source| AccessError::Caller {
        message: source.to_string(),
    })?;
    if caller.user_role == Some(UserRole::ProxyAdmin) {
        return Ok(());
    }
    Err(AccessError::WriterForbidden {
        action: action.to_owned(),
    })
}
