use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;

use crate::key_delegation::caller::Caller;
use crate::key_delegation::error::DelegationError;
use crate::key_delegation::request::Sent;

// _NON_ADMIN_SAFE_ALLOWED_ROUTES_PRESETS: the buckets a key_type preset derives without an admin.
pub const SAFE_ALLOWED_ROUTE_PRESETS: [&str; 2] = ["llm_api_routes", "info_routes"];
// _READ_ONLY_ALLOWED_ROUTES_PRESET: a read-only key stays read-only, so widening it needs an admin.
pub const READ_ONLY_ALLOWED_ROUTE_PRESET: &str = "info_routes";

fn is_safe_preset(route: &JsonData) -> bool {
    match route {
        JsonData::String(name) => SAFE_ALLOWED_ROUTE_PRESETS.contains(&name.as_str()),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => false,
    }
}

fn same_preset_set(left: &[JsonData], right: &[JsonData]) -> bool {
    left.iter().all(|route| right.contains(route)) && right.iter().all(|route| left.contains(route))
}

// Only a proxy admin sets allowed_routes, unless a key_type preset derived every route the caller sent.
pub fn check_routes_permission(
    allowed_routes: &Sent<Vec<JsonData>>,
    allow_safe_presets: bool,
    caller: Caller,
) -> Result<(), DelegationError> {
    let routes = allowed_routes.value.as_deref();
    if !allowed_routes.supplied && routes.is_none_or(<[JsonData]>::is_empty) {
        return Ok(());
    }
    if caller.role == Some(UserRole::ProxyAdmin) {
        return Ok(());
    }
    if allow_safe_presets
        && routes.is_some_and(|list| !list.is_empty() && list.iter().all(is_safe_preset))
    {
        return Ok(());
    }
    Err(DelegationError::AllowedRoutes)
}

// Both sides must hold only safe preset buckets, and an existing read-only bucket may only stay the same set.
#[must_use]
pub fn is_safe_route_transition(
    incoming: Option<&[JsonData]>,
    existing: Option<&[JsonData]>,
) -> bool {
    let incoming = incoming.unwrap_or_default();
    let existing = existing.unwrap_or_default();
    if !incoming.iter().chain(existing).all(is_safe_preset) {
        return false;
    }
    let read_only = !existing.is_empty()
        && existing.iter().all(|route| {
            matches!(route, JsonData::String(name) if name.as_str() == READ_ONLY_ALLOWED_ROUTE_PRESET)
        });
    !read_only || same_preset_set(incoming, existing)
}

// An unsafe transition falls back to the raw-body check, which keeps the caller's model_fields_set.
pub fn enforce_routes_update(
    allowed_routes: &Sent<Vec<JsonData>>,
    existing_allowed_routes: Option<&[JsonData]>,
    caller: Caller,
) -> Result<(), DelegationError> {
    if is_safe_route_transition(allowed_routes.value.as_deref(), existing_allowed_routes) {
        return Ok(());
    }
    check_routes_permission(allowed_routes, false, caller)
}
