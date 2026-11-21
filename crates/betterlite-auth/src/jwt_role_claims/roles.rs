use betterlite_config::sections::jwt_settings::Settings;
use betterlite_paths::paths::lookup::{find, find_or};
use betterlite_paths::paths::{Document, PathLimits};

use super::error::RoleClaimError;
use super::members::{Member, members};
use crate::jwt_claims::TypedValue;

const ROLE_PATH_LIMITS: PathLimits = PathLimits {
    max_path_bytes: 8192,
    max_depth: 256,
    max_selectors: 256,
    max_visits: 100_000,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleField {
    UserRoles,
    Roles,
}

// an unset field or a missing path gives the default; a found value keeps its shape unless a list default asks for a list
pub fn role_claim(
    claims: &TypedValue,
    settings: &Settings,
    field: RoleField,
    default: Option<&[String]>,
) -> Result<Option<TypedValue>, RoleClaimError> {
    let configured = match field {
        RoleField::UserRoles => settings.claims.user_roles_jwt_field.as_deref(),
        RoleField::Roles => settings.rbac.roles_jwt_field.as_deref(),
    };
    let fallback = default.map(|roles| Document {
        json: serde_json::json!(roles),
    });
    let Some(path) = configured else {
        return Ok(fallback);
    };
    let path_error = |source| RoleClaimError::Path {
        claim: path.to_owned(),
        source,
    };
    let found = match fallback {
        Some(list) => Some(find_or(&ROLE_PATH_LIMITS, claims, path, list).map_err(path_error)?),
        None => find(&ROLE_PATH_LIMITS, claims, path).map_err(path_error)?,
    };
    Ok(found.filter(|value| !value.json.is_null()))
}

// is_allowed_user_role: any member of the claim, as Python iterates it, equal to an allowed role
pub fn is_allowed_user_role(
    user_roles: Option<&TypedValue>,
    settings: &Settings,
) -> Result<bool, RoleClaimError> {
    let Some(claim) = user_roles.filter(|value| !value.json.is_null()) else {
        return Ok(false);
    };
    let Some(allowed) = settings.claims.user_allowed_roles.as_ref() else {
        return Ok(false);
    };
    let found = members(claim)?;
    Ok(found
        .iter()
        .any(|member| matches!(member, Member::Text(role) if allowed.contains(role))))
}
