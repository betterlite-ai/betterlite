use betterlite_db::identity_rows::user::LiteLLMUserTable;
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

#[must_use]
pub fn map_role(role: Option<&str>) -> Option<Role> {
    match role {
        Some("proxy_admin") => Some(Role::PlatformAdmin),
        Some("proxy_admin_viewer") => Some(Role::PlatformViewer),
        Some("org_admin") => Some(Role::OrgAdmin),
        _ => None,
    }
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
        _ => UserRole::InternalUser,
    }
}

#[must_use]
pub fn get_user_role(user: Option<&LiteLLMUserTable>) -> Option<UserRole> {
    let row = user?;
    let role = effective_user_role(row.user_role.as_deref());
    Some(role)
}

#[must_use]
pub fn is_user_proxy_admin(user: Option<&LiteLLMUserTable>) -> bool {
    let Some(row) = user else {
        return false;
    };
    row.user_role.as_deref() == Some("proxy_admin")
}

mod tests;
