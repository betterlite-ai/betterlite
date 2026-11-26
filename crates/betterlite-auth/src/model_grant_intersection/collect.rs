use std::collections::BTreeSet;

use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;

use crate::auth_prefetch::rows::CachedProject;
use crate::model_grant_intersection::error::IntersectionError;
use crate::model_grant_intersection::grant::{AttributionRequest, granted_model_lists};
use crate::model_grant_intersection::serving::serving_groups;
use crate::model_grant_intersection::sources::{GrantSources, ServingGroups};
use crate::model_grant_intersection::token::GrantToken;

// collect_matched_model_access_groups' inputs: the request's model(s) and every loaded object and collaborator.
pub struct MatchedRequest<'request> {
    pub models: Option<&'request [String]>,
    pub token: Option<&'request GrantToken>,
    pub team: Option<&'request LiteLLMTeamTableCachedObj>,
    pub project: Option<&'request CachedProject>,
    pub membership: Option<LiteLLMTeamMembership>,
    pub membership_loaded: bool,
    pub groups: Option<&'request dyn ServingGroups>,
    pub sources: Option<&'request dyn GrantSources>,
}

// collect_matched_model_access_groups: the budgeted groups named on a granted allowlist that serve the request.
pub async fn collect_matched_model_access_groups(
    request: MatchedRequest<'_>,
) -> Result<Vec<String>, IntersectionError> {
    let (Some(models), Some(token), Some(groups), Some(sources)) = (
        request.models,
        request.token,
        request.groups,
        request.sources,
    ) else {
        return Ok(Vec::new());
    };
    let registry = sources.budget_groups().await?;
    if registry.as_ref().is_some_and(BTreeSet::is_empty) {
        return Ok(Vec::new());
    }
    let covering = serving_groups(models, groups, token.team_id.as_deref()).await?;
    let budgeted = match &registry {
        Some(known) => covering.intersection(known).cloned().collect(),
        None => covering,
    };
    if budgeted.is_empty() {
        return Ok(Vec::new());
    }
    let lists = granted_model_lists(AttributionRequest {
        token,
        team: request.team,
        project: request.project,
        membership: request.membership,
        membership_loaded: request.membership_loaded,
        sources,
    })
    .await?;
    let mut granted = BTreeSet::new();
    for list in lists {
        granted.extend(list);
    }
    Ok(budgeted.intersection(&granted).cloned().collect())
}
