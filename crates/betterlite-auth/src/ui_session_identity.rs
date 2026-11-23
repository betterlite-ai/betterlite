use async_trait::async_trait;

use crate::auth_user_lookup::error::UserLookupError;
use crate::resolved_key::auth::UserAPIKeyAuth;

mod user_lookup;

mod tests;

// constants.py's UI_SESSION_TOKEN_TEAM_ID: the team a dashboard-minted session token acts under
pub const UI_SESSION_TOKEN_TEAM_ID: &str = "litellm-dashboard";

// the one read the resolver makes: whether a database exists, and a user's team list
#[async_trait]
pub trait UiSessionTeams: Send + Sync {
    fn has_database(&self) -> bool;
    async fn user_teams(
        &self,
        user_id: &str,
        fresh: bool,
    ) -> Result<Option<Vec<String>>, UserLookupError>;
}

// ui_session_utils.py's is_ui_session_credential: the dashboard team plus a signed-in user
#[must_use]
pub fn is_ui_session_credential(auth: &UserAPIKeyAuth) -> bool {
    auth.view.token.team_id.as_deref() == Some(UI_SESSION_TOKEN_TEAM_ID)
        && auth
            .view
            .token
            .user_id
            .as_deref()
            .is_some_and(|user_id| !user_id.is_empty())
}

pub async fn resolve_ui_session_team_ids<T>(auth: &UserAPIKeyAuth, teams: &T) -> Vec<String>
where
    T: UiSessionTeams + ?Sized,
{
    if !is_ui_session_credential(auth) {
        return Vec::new();
    }
    let Some(user_id) = auth.view.token.user_id.as_deref() else {
        return Vec::new();
    };
    if !teams.has_database() {
        tracing::debug!("Cannot resolve UI session team ids without DB access");
        return Vec::new();
    }
    match teams
        .user_teams(user_id, auth.extra.requires_fresh_policy)
        .await
    {
        Ok(Some(rows)) => unique_team_ids(rows),
        Ok(None) => Vec::new(),
        Err(error) => {
            tracing::warn!(
                error = %error,
                "Failed to load teams for UI session token user."
            );
            Vec::new()
        }
    }
}

// LiteLLM appends each nonempty team once, in the order the user row lists it
fn unique_team_ids(team_ids: Vec<String>) -> Vec<String> {
    let mut resolved: Vec<String> = Vec::with_capacity(team_ids.len());
    for team_id in team_ids {
        if !team_id.is_empty() && !resolved.contains(&team_id) {
            resolved.push(team_id);
        }
    }
    resolved
}
