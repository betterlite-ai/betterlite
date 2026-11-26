use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;

use crate::auth_prefetch::rows::CachedProject;

// A declaration is list[str]: a non-text entry grants nothing, since no allowlist check matches it.
#[must_use]
pub fn text_models(values: &[JsonData]) -> Vec<String> {
    let mut models = Vec::new();
    for value in values {
        if let JsonData::String(text) = value {
            models.push(text.clone());
        }
    }
    models
}

#[must_use]
pub fn team_models(team: Option<&LiteLLMTeamTableCachedObj>) -> Vec<String> {
    match team {
        Some(object) => object.team.fields.base.models.clone(),
        None => Vec::new(),
    }
}

#[must_use]
pub fn user_models(user: Option<&LiteLLMUserTable>) -> Vec<String> {
    match user {
        Some(row) => text_models(&row.models),
        None => Vec::new(),
    }
}

#[must_use]
pub fn project_models(project: Option<&CachedProject>) -> Vec<String> {
    match project {
        Some(cached) => cached.project.models.clone(),
        None => Vec::new(),
    }
}
