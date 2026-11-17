use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_auth::resolved_key::auth::user_api_key_has_admin_view;

use crate::management_guards::caller::caller_user_id_for_non_admin;
use crate::management_guards::error::ManagementGuardError;

const MASTER_KEY_SUBJECT: &str = "master_key";

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
    if has_admin_view(principal) {
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

// LiteLLM's user_api_key_has_admin_view, plus the master key, which carries no key record to read a role from
fn has_admin_view(principal: &Principal) -> bool {
    principal.subject == MASTER_KEY_SUBJECT
        || principal
            .source_key
            .0
            .as_ref()
            .is_some_and(user_api_key_has_admin_view)
        || principal.roles.contains(&Role::PlatformAdmin)
        || principal.roles.contains(&Role::PlatformViewer)
}
