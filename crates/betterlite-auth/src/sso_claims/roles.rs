use crate::jwt_claims::TypedValue;
use crate::jwt_claims::claims::ClaimError;
use crate::sso_types::records::RoleMappings;
use crate::sso_types::roles::{highest_privilege_role, role_from_claim};
use crate::user_roles::UserRole;

use super::claims::{claim_groups, present};

// determine_role_from_groups: the highest ranked mapped role one of the user's groups names, else the default role
#[must_use]
pub fn determine_role_from_groups(groups: &[String], mappings: &RoleMappings) -> Option<UserRole> {
    let matched = mappings
        .roles
        .iter()
        .filter(|(_, mapped_groups)| mapped_groups.iter().any(|group| groups.contains(group)))
        .map(|(role, _groups)| *role);
    highest_privilege_role(matched)
        .filter(|role| {
            // the ranked roles; the order among them is sso_types::roles::highest_privilege_role's
            matches!(
                role,
                UserRole::ProxyAdmin
                    | UserRole::ProxyAdminViewer
                    | UserRole::InternalUser
                    | UserRole::InternalUserViewer
            )
        })
        .or(mappings.default_role)
}

// the role a claim set names: an applicable group mapping decides first, then the direct role claim
pub(super) fn resolve_role(
    response: &TypedValue,
    role_mappings: Option<&RoleMappings>,
    role_attribute: &str,
) -> Result<Option<UserRole>, ClaimError> {
    if let Some(mappings) = role_mappings {
        let groups = claim_groups(response, &mappings.group_claim)?;
        let mapped = if groups.is_empty() {
            mappings.default_role
        } else {
            determine_role_from_groups(&groups, mappings)
        };
        if mapped.is_some() {
            return Ok(mapped);
        }
    }
    Ok(present(response, role_attribute)?
        .as_ref()
        .and_then(role_from_claim))
}
