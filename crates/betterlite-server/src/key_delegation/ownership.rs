use betterlite_auth::user_roles::UserRole;

use crate::key_delegation::caller::Caller;
use crate::key_delegation::error::DelegationError;
use crate::key_delegation::request::KeyDelegationRequest;

// A non-admin cannot detach or rebind a key's user, which would change the role resolved on later requests.
pub fn validate_ownership_change(
    request: Option<&KeyDelegationRequest>,
    existing_user_id: Option<&str>,
    caller: Caller,
) -> Result<(), DelegationError> {
    if caller.role == Some(UserRole::ProxyAdmin) {
        return Ok(());
    }
    let Some(request) = request else {
        return Ok(());
    };
    if !request.user_id.supplied {
        return Ok(());
    }
    let Some(incoming) = request.user_id.value.as_deref() else {
        return Err(DelegationError::RemoveUserId);
    };
    if incoming.is_empty() {
        return Err(DelegationError::RemoveUserId);
    }
    if Some(incoming) != existing_user_id {
        return Err(DelegationError::RebindUserId {
            existing: existing_user_id.map_or_else(|| "none".to_owned(), str::to_owned),
            incoming: incoming.to_owned(),
        });
    }
    Ok(())
}
