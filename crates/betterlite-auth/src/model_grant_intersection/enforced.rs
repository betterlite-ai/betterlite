use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;

use crate::auth_identity::errors::AuthError as IdentityError;
use crate::auth_model_patterns::grant::resolve_key_models;
use crate::auth_user_lookup::error::UserLookupError;
use crate::model_grant_intersection::error::IntersectionError;
use crate::model_grant_intersection::lists::{project_models, team_models, user_models};
use crate::model_grant_intersection::member::team_member_granted_models;
use crate::model_grant_intersection::sources::GrantSources;
use crate::model_grant_intersection::token::GrantToken;

// enforced_model_allowlists' inputs: the key, the database side (None for no DB) and the clock.
pub struct EnforcedRequest<'request> {
    pub token: &'request GrantToken,
    pub sources: Option<&'request dyn GrantSources>,
    pub now: f64,
}

// enforced_model_allowlists: key, team, member, user, project; the organization is not an enforcement level.
pub async fn enforced_model_allowlists(
    request: EnforcedRequest<'_>,
) -> Result<Vec<Vec<String>>, IntersectionError> {
    let key_list = resolve_key_models(
        &request.token.models,
        &request.token.team_models,
        request.token.team_id.as_deref(),
    );
    let Some(sources) = request.sources else {
        return Ok(vec![key_list, request.token.team_models.clone()]);
    };
    let team = match request.token.team_id.as_deref() {
        Some(team_id) => Some(sources.team_object(team_id).await?),
        None => None,
    };
    let user = user_object_or_none(request.token, team.as_ref(), sources).await?;
    let project = match request.token.project_id.as_deref() {
        Some(project_id) => sources.project_object(project_id, request.now).await?,
        None => None,
    };
    let member =
        team_member_granted_models(request.token, team.as_ref(), None, false, sources).await?;
    Ok(vec![
        key_list,
        team_models(team.as_ref()),
        member,
        user_models(user.as_ref()),
        project_models(project.as_ref()),
    ])
}

// _user_object_or_none: only UserNotFoundError is absence; every other failure escapes the walk.
async fn user_object_or_none(
    token: &GrantToken,
    team: Option<&LiteLLMTeamTableCachedObj>,
    sources: &dyn GrantSources,
) -> Result<Option<LiteLLMUserTable>, IntersectionError> {
    if team.is_some() {
        return Ok(None);
    }
    let Some(user_id) = token.user_id.as_deref() else {
        return Ok(None);
    };
    match sources.user_object(user_id).await {
        Ok(user) => Ok(user),
        Err(UserLookupError::NotFound(IdentityError::UserNotFound { .. })) => Ok(None),
        Err(error) => Err(IntersectionError::User(error)),
    }
}
