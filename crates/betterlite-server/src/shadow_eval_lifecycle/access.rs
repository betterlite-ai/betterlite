use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;

use crate::plugin_routes::routes::Identity;
use crate::shadow_eval_lifecycle::error::LifecycleError;
use crate::shadow_eval_lifecycle::store::ShadowLifecycleStore;
use crate::shadow_eval_validation::access::{AdminRole, require_admin as require_admin_key};
use crate::shadow_eval_validation::error::AccessError;

// _require_admin_viewer and _require_admin_writer over the UserAPIKeyAuth B345 reads, built from the identity the auth layer published
pub fn require_admin(
    identity: &Identity,
    action: &str,
    role: AdminRole,
) -> Result<(), AccessError> {
    match caller_key(identity) {
        Ok(key) => require_admin_key(&key, action, role),
        // a key view the typed model rejects carries no admin role, the way get_user_role reads an unknown one
        Err(_invalid) => Err(forbidden(action, role)),
    }
}

fn caller_key(identity: &Identity) -> Result<UserAPIKeyAuth, AuthError> {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    if let Some(user_id) = &identity.user_id {
        fields.insert("user_id".to_owned(), JsonData::String(user_id.clone()));
    }
    if let Some(user_role) = &identity.user_role {
        fields.insert("user_role".to_owned(), JsonData::String(user_role.clone()));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields))
}

fn forbidden(action: &str, role: AdminRole) -> AccessError {
    match role {
        AdminRole::Writer => AccessError::WriterForbidden {
            action: action.to_owned(),
        },
        AdminRole::Viewer => AccessError::ViewerForbidden {
            action: action.to_owned(),
        },
    }
}

// the start and stop route's guard order: a non-writer answers 403 before a proxy without a database answers 500
pub fn require_writer<'store, Store>(
    identity: &Identity,
    action: &str,
    store: Option<&'store Store>,
) -> Result<&'store Store, LifecycleError>
where
    Store: ShadowLifecycleStore + ?Sized,
{
    require_admin(identity, action, AdminRole::Writer)?;
    store.ok_or(LifecycleError::DatabaseUnavailable)
}
