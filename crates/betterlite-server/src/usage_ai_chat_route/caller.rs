use betterlite_auth::auth_identity::records::Principal;

use crate::management_guards::caller::{caller_has_admin_view, caller_user_id_for_non_admin};
use crate::management_guards::error::ManagementGuardError;

pub(super) struct CallerIdentity {
    pub(super) user_id: Option<String>,
    pub(super) is_admin: bool,
}

pub trait NonAdminCallerId: Send + Sync {
    fn user_id_for_non_admin<'principal>(
        &self,
        principal: &'principal Principal,
    ) -> Result<&'principal str, ManagementGuardError>;
}

pub struct GuardCallerId;

impl NonAdminCallerId for GuardCallerId {
    fn user_id_for_non_admin<'principal>(
        &self,
        principal: &'principal Principal,
    ) -> Result<&'principal str, ManagementGuardError> {
        caller_user_id_for_non_admin(principal)
    }
}

pub(super) fn identity(
    principal: &Principal,
    caller_id: &dyn NonAdminCallerId,
) -> Result<CallerIdentity, ManagementGuardError> {
    if caller_has_admin_view(principal) {
        let user_id = principal.user.as_ref().map(|user| user.id.clone());
        return Ok(CallerIdentity {
            user_id,
            is_admin: true,
        });
    }
    let user_id = caller_id.user_id_for_non_admin(principal)?;
    Ok(CallerIdentity {
        user_id: Some(user_id.to_owned()),
        is_admin: false,
    })
}
