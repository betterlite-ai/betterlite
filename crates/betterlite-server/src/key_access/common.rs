use betterlite_auth::user_roles::UserRole;

use super::error::KeyAccessError;
use crate::key_fields::base::KeyRequestBase;
use crate::team_access::caller::TeamCaller;

// _check_model_access_group's premium-user 403 is dropped (no paid tier), so a wildcard access group passes like any model
pub fn common_key_access_checks(
    caller: &TeamCaller,
    data: &KeyRequestBase,
    user_id: Option<&str>,
) -> Result<(), KeyAccessError> {
    if caller.user_role == Some(UserRole::ProxyAdmin) {
        return Ok(());
    }
    let requested = user_id.or_else(|| data.base.user_id.as_ref().and_then(Option::as_deref));
    if let Some(owner) = requested
        && caller.user_id.as_deref() != Some(owner)
    {
        return Err(KeyAccessError::NotOwner {
            user_id: owner.to_owned(),
        });
    }
    Ok(())
}
