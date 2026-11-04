use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use serde::{Deserialize, Serialize};

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
pub fn resolve_resource_owner_id(principal: &Principal) -> Option<String> {
    if let Some(user) = &principal.user {
        return Some(user.id.clone());
    }
    if !principal.teams.is_empty() {
        return None;
    }
    principal
        .credential_ref
        .token_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            principal
                .credential_ref
                .key_id
                .as_deref()
                .filter(|value| !value.is_empty())
        })
        .map(|value| format!("key:{value}"))
}

#[must_use]
pub fn build_owner_filter(principal: &Principal) -> Option<OwnerFilter> {
    if has_admin_view(principal) {
        return Some(OwnerFilter::Admin);
    }
    let user_id = principal.user.as_ref().map(|user| &user.id);
    let team_id = principal.teams.first().map(|team| &team.id);
    match (user_id, team_id) {
        (Some(owner_id), Some(owner_team_id)) => Some(OwnerFilter::UserOrTeam {
            user_id: owner_id.clone(),
            team_id: owner_team_id.clone(),
        }),
        (_, Some(owner_team_id)) => Some(OwnerFilter::TeamId {
            team_id: owner_team_id.clone(),
        }),
        _ => resolve_resource_owner_id(principal)
            .map(|created_by| OwnerFilter::CreatedBy { created_by }),
    }
}

#[must_use]
pub fn can_access_resource(
    principal: &Principal,
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
            .teams
            .first()
            .is_some_and(|team| resource_team_id == Some(team.id.as_str()))
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

fn has_admin_view(principal: &Principal) -> bool {
    principal.roles.iter().any(|role| match role {
        Role::PlatformAdmin | Role::PlatformViewer => true,
        Role::OrgAdmin | Role::OrgViewer | Role::TeamAdmin | Role::TeamMember => false,
    })
}

#[path = "managed_resource_access/tests.rs"]
mod tests;
