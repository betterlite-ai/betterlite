use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;

use crate::auth_model_patterns::grant::resolve_key_models;
use crate::auth_prefetch::rows::CachedProject;
use crate::model_grant_intersection::error::IntersectionError;
use crate::model_grant_intersection::lists::text_models;
use crate::model_grant_intersection::member::team_member_granted_models;
use crate::model_grant_intersection::org::org_granted_models;
use crate::model_grant_intersection::sources::GrantSources;
use crate::model_grant_intersection::token::GrantToken;

// What _granted_model_lists reads: the key plus the objects common_checks already loaded for the request.
pub struct AttributionRequest<'request> {
    pub token: &'request GrantToken,
    pub team: Option<&'request LiteLLMTeamTableCachedObj>,
    pub project: Option<&'request CachedProject>,
    pub membership: Option<LiteLLMTeamMembership>,
    pub membership_loaded: bool,
    pub sources: &'request dyn GrantSources,
}

// _granted_model_lists: key, team, member, project, org; attribution never reads the user scope.
pub async fn granted_model_lists(
    request: AttributionRequest<'_>,
) -> Result<Vec<Vec<String>>, IntersectionError> {
    let key_list = resolve_key_models(
        &request.token.models,
        &request.token.team_models,
        request.token.team_id.as_deref(),
    );
    let team_list = request
        .team
        .map_or_else(Vec::new, |object| object.team.fields.base.models.clone());
    let member = team_member_granted_models(
        request.token,
        request.team,
        request.membership,
        request.membership_loaded,
        request.sources,
    )
    .await?;
    let project_list = request
        .project
        .map_or_else(Vec::new, |cached| cached.project.models.clone());
    let org = org_granted_models(request.token, request.team, request.sources).await;
    Ok(vec![key_list, team_list, member, project_list, org])
}
