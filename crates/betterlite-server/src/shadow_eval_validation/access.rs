use betterlite_auth::resolved_key::auth::{UserAPIKeyAuth, user_api_key_has_admin_view};
use betterlite_auth::user_roles::UserRole;

use crate::shadow_eval_validation::error::AccessError;
use crate::team_access::caller::TeamCaller;

#[derive(Clone, Copy)]
pub enum AdminRole {
    Viewer,
    Writer,
}

// _require_admin_viewer and _require_admin_writer: serving a listing needs either admin role, while starting a job needs the full admin role.
pub fn require_admin(
    auth: &UserAPIKeyAuth,
    action: &str,
    role: AdminRole,
) -> Result<(), AccessError> {
    match role {
        AdminRole::Viewer if user_api_key_has_admin_view(auth) => Ok(()),
        AdminRole::Writer => {
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
        AdminRole::Viewer => Err(AccessError::ViewerForbidden {
            action: action.to_owned(),
        }),
    }
}
