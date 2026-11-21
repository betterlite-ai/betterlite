use betterlite_config::sections::auth_user_lookup::RbacRole;
use betterlite_config::sections::jwt_settings::Settings;

use super::error::RoleClaimError;
use super::members::{Member, is_empty_claim, members};
use super::roles::{RoleField, is_allowed_user_role, role_claim};
use super::scopes::scopes;
use crate::jwt_claims::TypedValue;
use crate::jwt_claims::claims::JwtClaims;
use crate::jwt_claims::identity::IdentityClaims as _;
use crate::jwt_claims::teams::TeamClaims as _;

// JWTHandler.get_rbac_role: the scope and user-role claims are read before any identity can win
pub fn rbac_role(
    identity: &JwtClaims<'_>,
    settings: &Settings,
) -> Result<Option<RbacRole>, RoleClaimError> {
    let granted = scopes(&identity.claims)?;
    let admin = Some(settings.routes.admin_jwt_scope.as_str());
    let user_roles = role_claim(&identity.claims, settings, RoleField::UserRoles, None)?;
    if granted.iter().any(|scope| scope.json.as_str() == admin) {
        return Ok(Some(RbacRole::ProxyAdmin));
    }
    if identity
        .team_id(None)
        .map_err(RoleClaimError::Identity)?
        .is_some()
    {
        return Ok(Some(RbacRole::Team));
    }
    let user_id = identity.user_id(None).map_err(RoleClaimError::Identity)?;
    if user_id.is_some() || is_allowed_user_role(user_roles.as_ref(), settings)? {
        return Ok(Some(RbacRole::InternalUser));
    }
    mapped_role(&identity.claims, settings)
}

// _rbac_role_from_role_mapping: exact roles only, and the configured mapping order wins over the claim's
pub fn mapped_role(
    claims: &TypedValue,
    settings: &Settings,
) -> Result<Option<RbacRole>, RoleClaimError> {
    let Some(mappings) = settings.rbac.role_mappings.as_ref() else {
        return Ok(None);
    };
    let Some(claim) = role_claim(claims, settings, RoleField::Roles, None)? else {
        return Ok(None);
    };
    if is_empty_claim(&claim) {
        return Ok(None);
    }
    let roles = members(&claim)?;
    if roles.contains(&Member::Container) {
        return Err(RoleClaimError::Unhashable);
    }
    let found = mappings
        .iter()
        .find(|mapping| roles.contains(&Member::Text(mapping.role.clone())));
    Ok(found.map(|mapping| mapping.internal_role))
}
