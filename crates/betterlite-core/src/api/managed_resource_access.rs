use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuthenticatedPrincipal {
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub token: Option<SecretString>,
    pub api_key: Option<SecretString>,
    pub role: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OwnerFilter {
    CreatedBy { created_by: String },
    TeamId { team_id: String },
    UserOrTeam { user_id: String, team_id: String },
    Admin,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceRecord {
    pub id: String,
    pub created_by: Option<String>,
    pub team_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ListPage {
    pub object: String,
    pub data: Vec<ResourceRecord>,
    pub first_id: Option<String>,
    pub last_id: Option<String>,
    pub has_more: bool,
}

#[must_use]
pub fn resolve_resource_owner_id(principal: &AuthenticatedPrincipal) -> Option<String> {
    if let Some(user_id) = &principal.user_id {
        return Some(user_id.clone());
    }
    if principal.team_id.is_some() {
        return None;
    }
    let token = principal
        .token
        .as_ref()
        .map(SecretString::expose_secret)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            principal
                .api_key
                .as_ref()
                .map(SecretString::expose_secret)
                .filter(|value| !value.is_empty())
        });
    token.map(|value| format!("key:{value}"))
}

#[must_use]
pub fn build_owner_filter(principal: &AuthenticatedPrincipal) -> Option<OwnerFilter> {
    if has_admin_view(principal) {
        return Some(OwnerFilter::Admin);
    }
    match (&principal.user_id, &principal.team_id) {
        (Some(user_id), Some(team_id)) => Some(OwnerFilter::UserOrTeam {
            user_id: user_id.clone(),
            team_id: team_id.clone(),
        }),
        (_, Some(team_id)) => Some(OwnerFilter::TeamId {
            team_id: team_id.clone(),
        }),
        _ => resolve_resource_owner_id(principal)
            .map(|created_by| OwnerFilter::CreatedBy { created_by }),
    }
}

#[must_use]
pub fn can_access_resource(
    principal: &AuthenticatedPrincipal,
    created_by: Option<&str>,
    resource_team_id: Option<&str>,
) -> bool {
    if has_admin_view(principal) {
        return true;
    }
    resolve_resource_owner_id(principal)
        .as_deref()
        .is_some_and(|owner| created_by == Some(owner))
        || principal
            .team_id
            .as_deref()
            .is_some_and(|team| resource_team_id == Some(team))
}

#[must_use]
pub fn build_list_page(items: Vec<ResourceRecord>, has_more: bool) -> ListPage {
    let first_id = items.first().map(|item| item.id.clone());
    let last_id = items.last().map(|item| item.id.clone());
    ListPage {
        object: "list".to_owned(),
        data: items,
        first_id,
        last_id,
        has_more,
    }
}

fn has_admin_view(principal: &AuthenticatedPrincipal) -> bool {
    match principal.role.as_str() {
        "proxy_admin" | "proxy_admin_viewer" => true,
        _ => false,
    }
}

#[path = "managed_resource_access/tests.rs"]
mod tests;
