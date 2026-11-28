use betterlite_analytics::canonical_date::{ScopeDenial, ScopeDenied};
use betterlite_auth::auth_key_lookup::key_identity;
use betterlite_auth::resolved_key::auth::{UserAPIKeyAuth, user_api_key_has_admin_view};

use self::error::UserActivityScopeError;
use crate::management_guards::error::ManagementGuardError;

pub mod error;
mod tests;

// ast-grep-ignore: no-long-names
pub fn resolve_user_daily_activity_entity_ids(
    user_id: Option<&str>,
    auth: &UserAPIKeyAuth,
) -> Result<Option<Vec<String>>, UserActivityScopeError> {
    if user_api_key_has_admin_view(auth) {
        return Ok(user_id.map(|requested| vec![requested.to_owned()]));
    }
    let Some(caller_id) = key_identity(auth).user_id else {
        return Err(UserActivityScopeError::Caller(
            ManagementGuardError::ServiceAccountAnalytics,
        ));
    };
    if user_id.is_some_and(|requested| requested != caller_id.as_str()) {
        return Err(UserActivityScopeError::Denied(ScopeDenied {
            status: ScopeDenial::Forbidden,
            reason: "Non-admin users can only view their own spend data.".to_owned(),
        }));
    }
    Ok(Some(vec![caller_id]))
}
