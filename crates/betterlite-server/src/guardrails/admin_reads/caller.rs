use axum::extract::Extension;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_guardrails::guardrail_registry::rows::GuardrailRows;

use super::error::ReadError;
use super::state::ReadState;
use crate::auth::auth_identity::AuthContext;
use crate::plugin_routes::routes::caller_identity;

pub(super) use crate::management_guards::caller::caller_has_admin_view as has_admin_view;

pub(super) fn authenticated(
    caller: Option<Extension<AuthContext>>,
) -> Result<AuthContext, ReadError> {
    match caller {
        Some(Extension(context)) => Ok(context),
        None => Err(ReadError::Unauthenticated),
    }
}

// _get_user_team_ids: no user id or no database answers no teams, and empty strings are dropped
pub(super) async fn team_ids<R>(
    state: &ReadState<R>,
    principal: &Principal,
) -> Result<Vec<String>, ReadError>
where
    R: GuardrailRows,
{
    if !state.database {
        return Ok(Vec::new());
    }
    let identity = caller_identity(principal).map_err(|source| ReadError::Identity { source })?;
    let Some(user_id) = identity.user_id else {
        return Ok(Vec::new());
    };
    let teams = state
        .teams
        .team_ids(&user_id)
        .await
        .map_err(|source| ReadError::Teams { source })?;
    Ok(teams.into_iter().filter(|team| !team.is_empty()).collect())
}
