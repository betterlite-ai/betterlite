use serde::{Deserialize, Serialize};

use crate::user_roles::UserRole;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum AuthMethod {
    ApiKey,
    HttpBasic,
    BearerJwt,
    Oauth2Introspection,
    Oidc,
    Saml,
    MutualTls,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum PrincipalType {
    Human,
    ServiceAccount,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Role {
    PlatformAdmin,
    PlatformViewer,
    OrgAdmin,
    OrgViewer,
    TeamAdmin,
    TeamMember,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum TeamRole {
    Admin,
    #[default]
    Member,
}

const ROLE_MAP: &[(&str, Role)] = &[
    ("proxy_admin", Role::PlatformAdmin),
    ("proxy_admin_viewer", Role::PlatformViewer),
    ("org_admin", Role::OrgAdmin),
];

#[must_use]
pub fn map_role(role: Option<&str>) -> Option<Role> {
    let value = role?;
    ROLE_MAP
        .iter()
        .find_map(|(key, mapped)| (*key == value).then_some(*mapped))
}

#[must_use]
pub fn team_role(role: Option<&str>) -> TeamRole {
    if role == Some("admin") {
        TeamRole::Admin
    } else {
        TeamRole::Member
    }
}

#[must_use]
pub fn effective_user_role(role: Option<&str>) -> UserRole {
    match role {
        Some("proxy_admin") => UserRole::ProxyAdmin,
        Some("proxy_admin_viewer") => UserRole::ProxyAdminViewer,
        Some("org_admin") => UserRole::OrgAdmin,
        Some("internal_user_viewer") => UserRole::InternalUserViewer,
        Some("team") => UserRole::Team,
        Some("customer") => UserRole::Customer,
        Some(_) | None => UserRole::InternalUser,
    }
}

mod tests;
