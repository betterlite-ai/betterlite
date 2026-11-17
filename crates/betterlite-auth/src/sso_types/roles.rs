use crate::jwt_claims::TypedValue;
use crate::user_roles::UserRole;

const ROLE_HIERARCHY: [UserRole; 4] = [
    UserRole::ProxyAdmin,
    UserRole::ProxyAdminViewer,
    UserRole::InternalUser,
    UserRole::InternalUserViewer,
];

const ROLE_NAMES: [(&str, UserRole); 7] = [
    ("proxy_admin", UserRole::ProxyAdmin),
    ("proxy_admin_viewer", UserRole::ProxyAdminViewer),
    ("org_admin", UserRole::OrgAdmin),
    ("internal_user", UserRole::InternalUser),
    ("internal_user_viewer", UserRole::InternalUserViewer),
    ("team", UserRole::Team),
    ("customer", UserRole::Customer),
];

#[must_use]
pub fn highest_privilege_role<I>(roles: I) -> Option<UserRole>
where
    I: IntoIterator<Item = UserRole>,
{
    let resolved = roles.into_iter().collect::<Vec<UserRole>>();
    for ranked in ROLE_HIERARCHY {
        if resolved.contains(&ranked) {
            return Some(ranked);
        }
    }
    resolved.into_iter().min_by_key(ToString::to_string)
}

#[must_use]
pub fn is_valid_litellm_user_role(role: &str) -> bool {
    let lowered = role.to_lowercase();
    for (known, _role) in ROLE_NAMES {
        if lowered == known {
            return true;
        }
    }
    false
}

#[must_use]
pub fn role_from_claim(claim: &TypedValue) -> Option<UserRole> {
    let Some(members) = claim.json.as_array() else {
        return role_from_name(claim.json.as_str()?);
    };
    highest_privilege_role(
        members
            .iter()
            .filter_map(|member| role_from_name(member.as_str()?)),
    )
}

#[must_use]
pub fn role_from_name(name: &str) -> Option<UserRole> {
    let lowered = name.to_lowercase();
    for (known, role) in ROLE_NAMES {
        if lowered == known {
            return Some(role);
        }
    }
    None
}

#[must_use]
pub fn has_admin_ui_access(user_role: &str) -> bool {
    const ADMIN_UI_ROLES: [&str; 2] = ["proxy_admin", "proxy_admin_viewer"];
    ADMIN_UI_ROLES.contains(&user_role)
}
