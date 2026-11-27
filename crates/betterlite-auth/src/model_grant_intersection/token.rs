use crate::model_grant_intersection::lists::text_models;
use crate::resolved_key::auth::UserAPIKeyAuth;

// The identity fields the enforcement and attribution walks read, decoded once from the key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GrantToken {
    pub models: Vec<String>,
    pub team_models: Vec<String>,
    pub team_id: Option<String>,
    pub user_id: Option<String>,
    pub project_id: Option<String>,
    pub org_id: Option<String>,
}

impl GrantToken {
    #[must_use]
    pub fn of(auth: &UserAPIKeyAuth) -> Self {
        let token = &auth.view.token;
        Self {
            models: text_models(&token.models),
            team_models: text_models(&auth.view.team_models),
            team_id: token.team_id.clone(),
            user_id: token.user_id.clone(),
            project_id: token.project_id.clone(),
            org_id: token.org_id.clone(),
        }
    }
}
