use crate::sso_types::records::RoleMappings;
use crate::user_roles::UserRole;

// the ranking LiteLLM walks over group mappings; a role outside it never answers a group
const ROLE_HIERARCHY: [UserRole; 4] = [
    UserRole::ProxyAdmin,
    UserRole::ProxyAdminViewer,
    UserRole::InternalUser,
    UserRole::InternalUserViewer,
];

// determine_role_from_groups: the highest ranked mapped role one of the user's groups names, else the default role
#[must_use]
pub fn determine_role_from_groups(groups: &[String], mappings: &RoleMappings) -> Option<UserRole> {
    for role in ROLE_HIERARCHY {
        let Some((_, mapped_groups)) = mappings
            .roles
            .iter()
            .find(|(mapped, _groups)| *mapped == role)
        else {
            continue;
        };
        if mapped_groups.iter().any(|group| groups.contains(group)) {
            return Some(role);
        }
    }
    mappings.default_role
}
