use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;

use crate::auth_read::membership::member_allowed_models;
use crate::model_grant_intersection::error::IntersectionError;
use crate::model_grant_intersection::sources::GrantSources;
use crate::model_grant_intersection::token::GrantToken;

// _team_member_granted_models: the member's own allowed_models scope, empty when no member scope narrows the team.
pub async fn team_member_granted_models(
    token: &GrantToken,
    team: Option<&LiteLLMTeamTableCachedObj>,
    membership: Option<LiteLLMTeamMembership>,
    membership_loaded: bool,
    sources: &dyn GrantSources,
) -> Result<Vec<String>, IntersectionError> {
    let Some(team) = team else {
        return Ok(Vec::new());
    };
    let Some(user_id) = token.user_id.as_deref() else {
        return Ok(Vec::new());
    };
    let membership = if membership_loaded {
        membership
    } else {
        sources
            .membership(user_id, &team.team.fields.base.team_id)
            .await?
    };
    match membership {
        Some(row) => Ok(member_allowed_models(&row).to_vec()),
        None => Ok(Vec::new()),
    }
}
