use betterlite_config::sections::jwt_settings::Settings;
use betterlite_config::sections::jwt_settings::records::UserRole;

use super::error::{RoleClaimError, kind_name};
use super::fnmatch::RolePattern;
use super::roles::{RoleField, role_claim};
use crate::jwt_claims::TypedValue;

// map_jwt_role_to_litellm_role: each mapping in order scans the roles, and a role that is not text fails once reached
pub fn litellm_role(
    claims: &TypedValue,
    settings: &Settings,
) -> Result<Option<UserRole>, RoleClaimError> {
    let configured = settings.sync.jwt_litellm_role_map.as_ref();
    let Some(mappings) = configured.filter(|mappings| !mappings.is_empty()) else {
        return Ok(None);
    };
    let claim = role_claim(claims, settings, RoleField::Roles, Some(&[]))?;
    let Some(roles) = claim.as_ref().and_then(|value| value.json.as_array()) else {
        return Ok(None);
    };
    for mapping in mappings {
        let pattern = RolePattern::new(&mapping.jwt_role);
        for role in roles {
            let Some(text) = role.as_str() else {
                let found = kind_name(&TypedValue { json: role.clone() });
                return Err(RoleClaimError::NotText { found });
            };
            if pattern.matches(text) {
                return Ok(Some(mapping.litellm_role));
            }
        }
    }
    Ok(None)
}
