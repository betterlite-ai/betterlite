use axum::extract::Extension;
use betterlite_auth::auth_identity::records::Principal;
use serde::Deserialize;

use super::SubmissionsState;
use super::error::SubmissionError;
use super::rows::SubmissionRows;
use crate::auth::auth_identity::AuthContext;
use crate::plugin_routes::routes::caller_identity;

pub(super) use crate::management_guards::caller::caller_has_admin_view as has_admin_view;

// the user_api_key_dict fields B732's identity leaves out: the key's team and the caller's email
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct KeyScope {
    team_id: Option<String>,
    user_email: Option<String>,
}

#[derive(Debug)]
pub(super) struct Caller {
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub team_id: Option<String>,
    pub role: Option<String>,
}

pub(super) fn authenticated(
    caller: Option<Extension<AuthContext>>,
) -> Result<AuthContext, SubmissionError> {
    match caller {
        Some(Extension(context)) => Ok(context),
        None => Err(SubmissionError::Unauthenticated),
    }
}

// user_api_key_dict's identity: B732's user id and role, the key's team and the user row's email first
pub(super) fn caller(principal: &Principal) -> Result<Caller, SubmissionError> {
    let identity =
        caller_identity(principal).map_err(|source| SubmissionError::Identity { source })?;
    let scope = key_scope(principal).map_err(|source| SubmissionError::Identity { source })?;
    let key_email = scope.as_ref().and_then(|key| key.user_email.clone());
    let user_email = principal
        .user
        .as_ref()
        .map_or(key_email, |user| user.email.clone());
    Ok(Caller {
        user_id: identity.user_id,
        user_email,
        team_id: scope.and_then(|key| key.team_id),
        role: identity.user_role,
    })
}

fn key_scope(principal: &Principal) -> Result<Option<KeyScope>, serde_json::Error> {
    let Some(key) = principal.source_key.0.as_ref() else {
        return Ok(None);
    };
    serde_json::from_value(serde_json::to_value(key)?).map(Some)
}

// _get_user_team_ids: no user id or no database answers no teams, and empty strings are dropped
pub(super) async fn user_team_ids<S>(
    caller: &Caller,
    state: &SubmissionsState<S>,
) -> Result<Vec<String>, SubmissionError>
where
    S: SubmissionRows,
{
    if state.database.is_none() {
        return Ok(Vec::new());
    }
    let Some(user_id) = caller.user_id.as_deref() else {
        return Ok(Vec::new());
    };
    let teams = state
        .users
        .team_ids(user_id)
        .await
        .map_err(|source| SubmissionError::Teams { source })?;
    Ok(teams.into_iter().filter(|team| !team.is_empty()).collect())
}
