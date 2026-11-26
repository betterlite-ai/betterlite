use std::collections::BTreeMap;

use serde::Deserialize;

pub const ORG_ADMIN: &str = "org_admin";
pub const PROXY_ADMIN: &str = "proxy_admin";

// the membership row the checks read; a null organization_id is skipped by the pinned code
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OrganizationMembership {
    #[serde(default)]
    pub organization_id: Option<String>,
    #[serde(default)]
    pub user_role: Option<String>,
}

// LiteLLM_UserTable as the role checks read it; None memberships and Some(empty) both mean "no organization".
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RouteUser {
    #[serde(default)]
    pub user_role: Option<String>,
    #[serde(default)]
    pub organization_memberships: Option<Vec<OrganizationMembership>>,
}

// get_user_organization_info's two return values: the ids in membership order with duplicates kept, and the last role each id stored
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OrganizationInfo {
    pub organizations: Vec<String>,
    pub roles: BTreeMap<String, Option<String>>,
}

impl RouteUser {
    #[must_use]
    pub fn organization_info(&self) -> OrganizationInfo {
        let mut info = OrganizationInfo::default();
        let Some(memberships) = &self.organization_memberships else {
            return info;
        };
        for membership in memberships {
            let Some(organization_id) = &membership.organization_id else {
                continue;
            };
            info.organizations.push(organization_id.clone());
            info.roles
                .insert(organization_id.clone(), membership.user_role.clone());
        }
        info
    }
}
