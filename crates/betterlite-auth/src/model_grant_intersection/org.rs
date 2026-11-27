use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use tracing::debug;

use crate::model_grant_intersection::sources::GrantSources;
use crate::model_grant_intersection::token::GrantToken;

// _org_granted_models: the key's org, or its team's, and a failed lookup attributes nothing.
pub async fn org_granted_models(
    token: &GrantToken,
    team: Option<&LiteLLMTeamTableCachedObj>,
    sources: &dyn GrantSources,
) -> Vec<String> {
    let team_org = team.and_then(|object| object.team.fields.base.organization_id.as_deref());
    let Some(org_id) = token.org_id.as_deref().or(team_org) else {
        return Vec::new();
    };
    match sources.organization(org_id).await {
        Ok(Some(org)) => org.models,
        Ok(None) => Vec::new(),
        Err(error) => {
            debug!("access group attribution: org lookup failed: {error}");
            Vec::new()
        }
    }
}
