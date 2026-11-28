use std::sync::Arc;

use axum::extract::Extension;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_models::source::CatalogSnapshot;

use crate::auth::auth_identity::AuthContext;
use crate::management_guards::caller::caller_has_admin_view;
use crate::plugin_routes::routes::{PROXY_ADMIN_ROLE, caller_identity};
use crate::reload_routes::ReloadRoutesState;

use super::error::ReloadError;
use super::ports::ReloadConfigStore;

pub(super) fn caller_principal(
    caller: Option<Extension<AuthContext>>,
) -> Result<Principal, ReloadError> {
    caller
        .map(|extension| extension.0.principal)
        .ok_or(ReloadError::Unauthenticated)
}

pub(super) fn user_role(principal: &Principal) -> String {
    match caller_identity(principal) {
        Ok(identity) => identity.user_role.unwrap_or_default(),
        // a key always serializes, so no caller reaches this arm
        Err(_source) => String::new(),
    }
}

pub(super) fn require_proxy_admin(principal: &Principal) -> Result<(), ReloadError> {
    let role = user_role(principal);
    if role == PROXY_ADMIN_ROLE {
        return Ok(());
    }
    Err(ReloadError::Forbidden { role })
}

pub(super) fn require_admin_view(principal: &Principal) -> Result<(), ReloadError> {
    if caller_has_admin_view(principal) {
        return Ok(());
    }
    Err(ReloadError::Forbidden {
        role: user_role(principal),
    })
}

// the write handlers' database check, which LiteLLM's catch turns into a wrapped 500
pub(super) fn write_store<'state>(
    state: &'state ReloadRoutesState,
    prefix: &'static str,
) -> Result<&'state dyn ReloadConfigStore, ReloadError> {
    state
        .config
        .as_deref()
        .ok_or_else(|| ReloadError::missing_database(prefix))
}

// the two read handlers start alike: an admin-view caller reads the pod's current snapshot
pub(super) fn admin_read_snapshot(
    state: &ReloadRoutesState,
    caller: Option<Extension<AuthContext>>,
) -> Result<Arc<CatalogSnapshot>, ReloadError> {
    let principal = caller_principal(caller)?;
    require_admin_view(&principal)?;
    Ok(state.catalog.current())
}
